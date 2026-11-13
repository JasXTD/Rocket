//! Bounded PR reads use the account matched to this repository, never a global
//! fallback. Each read is tied to its session and repo; stale answers are discarded.
use super::*;
use crate::{
    backend::Backend,
    i18n::{mark, tr, trf, trn},
};
mod reviews;
use reviews::{ReviewMark, ReviewRead, review_mark, review_strip, reviewer_filter};
const MAX_PAGES: usize = 10;
/// From this width of the page's content (the page less its margins) the list and
/// the details sit side by side: the narrowest list that keeps its rows readable
/// beside the narrowest details card.
const SIDE_BY_SIDE: f32 = 880.0;
/// From this content width the five figures share one row, each still wide enough
/// for its label; below it they take two.
const FIVE_ACROSS: f32 = 720.0;
/// In the list, where Waiting for review and Time to first review join them, the seven
/// share one row from this width; below it they take 4 + 3, and below [`FOUR_ACROSS`]
/// 3 + 2 + 2.
const SEVEN_ACROSS: f32 = 1100.0;
const FOUR_ACROSS: f32 = 560.0;
/// From this width the author and reviewer fields fit on the row beside the period and
/// the List | Trends switch; below it they take a row of their own.
const FIELDS_INLINE: f32 = 720.0;
/// Pull requests whose reviews one Read reviews reads, newest first.
const REVIEW_BATCH: usize = 100;
const REVIEWS_STOPPED: &str =
    mark("Stopped. The reviews already read are kept; Read reviews continues.");
/// The least height of the list and details side by side, enough for a card's
/// header and some of its tab; a shorter page scrolls.
const SPLIT_MIN: f32 = 360.0;
const BAR: f32 = 40.0;
const ROW: f32 = 56.0;
pub(super) type Connection = (ForgeRepository, Arc<ForgeSession>);

/// Why `error` is set, for how the page shows it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Trouble {
    #[default]
    Clear,
    NoAccount,
    Changed,
    Stopped,
    SignIn,
    Failed,
}

#[derive(Default)]
pub struct State {
    pub items: Vec<ForgeItem>,
    pub metrics: ForgePullMetrics,
    pub pages: usize,
    pub error: Option<String>,
    pub loaded: bool,
    pub attempted: bool,
    pub limited: bool,
    pub more: bool,
    pub want_more: bool,
    pub stopping: bool,
    pub read_at: u64,
    pub period: Option<Period>,
    /// Rows shown: 0 all, 1 open, 2 merged, 3 waiting for review.
    pub show: u8,
    pub author: Option<String>,
    /// The chosen reviewer: the numbers and rows are the pull requests that name them.
    pub reviewer: Option<ForgeReviewer>,
    /// Reviews read for this sample, by pull request number. A new sample clears them.
    pub reviews: HashMap<u64, ForgeReviewRead>,
    pub review_metrics: ForgeReviewMetrics,
    /// Why the last Read reviews ended early as a whole, if it did.
    pub review_error: Option<String>,
    /// Trends instead of the list and its details.
    pub trends: bool,
    /// The chosen author's pull requests per week (or month) over the period.
    pub trend: ForgePullTrend,
    /// The same counts for the period before the chosen one; `None` for all history.
    pub previous: Option<ForgePullCounts>,
    /// How far back the loaded sample holds every pull request.
    pub coverage: ForgePullCoverage,
    pub detail: super::pull_detail::State,
    pub repository: Option<String>,
    trouble: Trouble,
    review_trouble: Trouble,
    connection: Option<Connection>,
    /// The host the sample was read from, for how its list is sorted and what it can
    /// tell about reviews.
    pub provider: Option<ForgeProvider>,
    task: Option<ReadTask<ForgePage>>,
    review_task: Option<ReviewRead>,
    next: Option<ForgeCursor>,
    append: bool,
}
fn same(a: &Connection, b: &Connection) -> bool {
    a.0 == b.0 && Arc::ptr_eq(&a.1, &b.1)
}

/// Whether the tiles and the Trends view can set the chosen period beside the one
/// before it.
pub(super) enum Comparison<'a> {
    /// All history has no period before it.
    None,
    Ready(&'a ForgePullCounts),
    /// The list changed on the host while it was read.
    Changed,
    /// The sample doesn't reach the previous period; `Some` is its oldest update, from
    /// which on it holds every pull request.
    NotLoaded(Option<u64>),
}

/// Pages Load more can still add.
pub(super) fn more_left(s: &State) -> bool {
    s.more && s.pages < MAX_PAGES
}

/// Whether the sample's host lists pull requests newest-updated first, so a partial
/// sample can say how far back it is complete.
pub(super) fn lists_by_update(s: &State) -> bool {
    s.provider.is_some_and(list_sorted_by_update)
}

impl State {
    pub fn running(&self) -> bool {
        self.task.is_some() || self.review_task.is_some() || self.detail.running()
    }
    pub fn list_running(&self) -> bool {
        self.task.is_some()
    }
    pub fn cancel(&mut self) {
        self.detail.cancel();
        if let Some(task) = &self.task {
            self.stopping = true;
            task.cancel();
        }
        if let Some(read) = &mut self.review_task {
            read.stopping = true;
            read.task.cancel();
        }
    }
    /// Leaving the page: drop the reads (which cancels them) and the details, keep the
    /// sample, its filters and when it was read.
    pub(super) fn park(&mut self) {
        self.detail = Default::default();
        // Dropping Read reviews cancels it; what it already handed over stays.
        if let Some(read) = self.review_task.take() {
            let arrived = read
                .results
                .lock()
                .map(|mut results| std::mem::take(&mut *results))
                .unwrap_or_default();
            if !arrived.is_empty() {
                self.reviews.extend(arrived);
                self.recalculate();
            }
        }
        if self.task.take().is_some() {
            self.stopping = false;
            self.want_more = false;
            self.append = false;
            // A first read that never finished: the next visit reads again instead of
            // showing an empty tab.
            if !self.loaded {
                self.attempted = false;
            }
        }
    }
    /// Nothing read yet, with saved filters.
    pub(super) fn with_filters(period: Period, author: Option<String>, show: u8, trends: bool) -> Self {
        Self {
            period: Some(period),
            author,
            show,
            trends,
            ..Default::default()
        }
    }
    /// A fresh state for another account or repository that keeps the filters chosen.
    fn reset(&mut self) {
        *self = Self {
            period: self.period,
            show: self.show,
            author: self.author.take(),
            reviewer: self.reviewer.take(),
            trends: self.trends,
            attempted: true,
            ..Default::default()
        };
    }
    /// The start of the chosen period (Unix seconds), `None` for all history.
    fn since(&self) -> Option<u64> {
        let days = match self.period.unwrap_or(Period::Days30) {
            Period::Days30 => 30,
            Period::Days90 => 90,
            Period::Year => 365,
            Period::All => return None,
        };
        Some(self.read_at.saturating_sub(days * 86400))
    }
    /// The period before the chosen one, as its start and its end (exclusive).
    pub(super) fn previous_range(&self) -> Option<(u64, u64)> {
        let since = self.since()?;
        Some((since.saturating_sub(self.read_at - since), since))
    }
    /// Whether the periods can be compared, for automation: `ready`, `changed`,
    /// `not_loaded` or `none`.
    pub fn comparable(&self) -> &'static str {
        match self.comparison() {
            Comparison::Ready(_) => "ready",
            Comparison::Changed => "changed",
            Comparison::NotLoaded(_) => "not_loaded",
            Comparison::None => "none",
        }
    }
    pub(super) fn comparison(&self) -> Comparison<'_> {
        let (Some(previous), Some((start, _))) = (&self.previous, self.previous_range()) else {
            return Comparison::None;
        };
        if self.limited {
            Comparison::Changed
        } else if !self.coverage.covers(start) {
            Comparison::NotLoaded(match self.coverage {
                ForgePullCoverage::After(oldest) => Some(oldest),
                _ => None,
            })
        } else {
            Comparison::Ready(previous)
        }
    }
    fn fail(&mut self, trouble: Trouble, text: String) {
        self.error = Some(text);
        self.trouble = trouble;
    }
    pub(super) fn start(&mut self, connection: Option<Connection>, backend: Backend, ctx: Context) {
        self.detail = Default::default();
        if let Some(task) = self.task.take() {
            task.cancel();
        }
        self.attempted = true;
        self.stopping = false;
        self.error = None;
        self.trouble = Trouble::Clear;
        let Some(connection) = connection else {
            self.fail(
                Trouble::NoAccount,
                tr("Connect a Hosting account for this repository, then Refresh.").into(),
            );
            return;
        };
        if self
            .connection
            .as_ref()
            .is_none_or(|old| !same(old, &connection))
        {
            self.reset();
        }
        self.provider = Some(connection.0.config().provider());
        self.append = self.want_more && self.loaded;
        self.want_more = false;
        if !self.append {
            // A new sample makes the running Read reviews' numbers moot.
            if let Some(read) = self.review_task.take() {
                read.task.cancel();
            }
        }
        let cursor = if self.append {
            if self.pages >= MAX_PAGES {
                return;
            }
            let Some(cursor) = self.next.clone() else {
                return;
            };
            cursor
        } else {
            ForgeCursor::default()
        };
        self.repository = Some(connection.0.path().into());
        self.connection = Some(connection.clone());
        let (repository, session) = connection;
        match ReadTask::spawn_with_wake(
            move |cancel| {
                let result = backend.forge_request("insights-pulls", &session, |s| {
                    s.list(
                        &repository,
                        &ForgeFilter {
                            kind: ForgeKind::Pulls,
                            state: ForgeState::All,
                            reviewers: true,
                            ..Default::default()
                        },
                        cursor,
                        &cancel,
                    )
                });
                cancel.check()?;
                result
            },
            move || ctx.request_repaint(),
        ) {
            Ok(task) => self.task = Some(task),
            Err(e) => self.fail(Trouble::Failed, user_error(&e)),
        }
    }
    pub(super) fn poll(&mut self, current: Option<&Connection>) {
        if self
            .connection
            .as_ref()
            .is_some_and(|old| current.is_none_or(|new| !same(old, new)))
        {
            self.cancel();
            self.reset();
            self.fail(
                Trouble::Changed,
                tr("Hosting account or repository changed. Refresh to read the new source.").into(),
            );
            return;
        }
        self.detail.poll();
        self.poll_reviews();
        let Some(result) = self.task.as_mut().and_then(ReadTask::try_finish) else {
            return;
        };
        self.task = None;
        let result = if self.stopping {
            Err(Error::Cancelled)
        } else {
            result
        };
        self.stopping = false;
        match result {
            Ok(page) => {
                if !self.append {
                    self.items.clear();
                    self.pages = 0;
                    self.limited = false;
                    // Reviews belong to the sample they were read for.
                    self.reviews.clear();
                    self.review_error = None;
                    self.review_trouble = Trouble::Clear;
                }
                self.pages += 1;
                self.limited |= page.incomplete;
                for item in page.items {
                    if self.items.iter().any(|old| old.number == item.number) {
                        // Concurrent changes can shift offset pagination. Keep the
                        // first observed row and disclose the non-atomic sample.
                        self.limited = true;
                    } else {
                        self.items.push(item);
                    }
                }
                self.next = page.next;
                self.more = self.next.is_some();
                self.read_at = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map_or(0, |d| d.as_secs());
                self.loaded = true;
                self.recalculate();
            }
            Err(Error::Cancelled) => self.fail(Trouble::Stopped, tr(STOPPED).into()),
            Err(e) => {
                // A refused sign-in is fixed in Hosting; anything else by reading again.
                let trouble = match &e {
                    Error::Forge(
                        ForgeFailure::Unauthorized
                        | ForgeFailure::Forbidden
                        | ForgeFailure::IdentityUnverified
                        | ForgeFailure::AzureIdentityUnverified,
                    ) => Trouble::SignIn,
                    _ => Trouble::Failed,
                };
                self.fail(trouble, user_error(&e));
            }
        }
    }
    pub(super) fn recalculate(&mut self) {
        let since = self.since();
        let cohort = self
            .items
            .iter()
            .filter(|i| self.in_cohort(i))
            .cloned()
            .collect::<Vec<_>>();
        self.metrics = ForgePullMetrics::from_items(&cohort, since, self.read_at);
        self.review_metrics = ForgeReviewMetrics::from_items(
            &cohort,
            &self.reviews,
            self.reviewer.as_ref(),
            since,
            self.read_at,
        );
        self.trend = ForgePullTrend::from_items(&cohort, since, self.read_at);
        self.previous = self
            .previous_range()
            .map(|(start, end)| ForgePullCounts::between(&cohort, start, end, self.read_at));
        // The whole sample, not the author's part, says how far back it is complete.
        self.coverage = match self.provider {
            Some(provider) => ForgePullCoverage::of(&self.items, provider, self.more),
            None if !self.more => ForgePullCoverage::Complete,
            None => ForgePullCoverage::Unknown,
        };
    }
    fn matches_author(&self, item: &ForgeItem) -> bool {
        self.author.as_ref().is_none_or(|a| a == &item.author)
    }
    /// The rows the numbers count: the chosen author's, and in the list the chosen
    /// reviewer's. Trends and its comparison with the period before count every
    /// reviewer: on GitHub a reviewer's pull requests are known only as far as their
    /// reviews were read, and the period before is never read.
    fn in_cohort(&self, item: &ForgeItem) -> bool {
        self.matches_author(item) && (self.trends || self.matches_reviewer(item))
    }
    pub fn visible(&self) -> impl Iterator<Item = &ForgeItem> {
        self.items.iter().filter(|i| {
            self.in_cohort(i)
                && match self.show {
                    1 => i.timing.as_ref().is_some_and(|t| t.open),
                    2 => i.timing.as_ref().is_some_and(|t| t.is_merged),
                    3 => self.waits(i),
                    _ => true,
                }
        })
    }
    pub fn authors(&self) -> Vec<String> {
        self.items
            .iter()
            .map(|i| i.author.clone())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect()
    }
    pub(super) fn source(&self) -> Option<(&'static str, &str)> {
        self.connection
            .as_ref()
            .map(|(repo, _)| (repo.config().provider().key(), repo.config().server()))
    }
    fn filter_changed(&mut self) {
        self.recalculate();
        if self
            .detail
            .number
            .is_some_and(|n| !self.visible().any(|i| i.number == n))
        {
            self.detail = Default::default();
        }
    }
    /// How many of the chosen author's and reviewer's loaded pull requests each state
    /// filter lists: all, open, merged and waiting for review.
    fn state_counts(&self) -> [usize; 4] {
        let mut counts = [0; 4];
        for item in self.items.iter().filter(|i| self.in_cohort(i)) {
            let timing = item.timing.as_ref();
            counts[0] += 1;
            counts[1] += usize::from(timing.is_some_and(|t| t.open));
            counts[2] += usize::from(timing.is_some_and(|t| t.is_merged));
            counts[3] += usize::from(self.waits(item));
        }
        counts
    }
}
pub(super) fn duration(seconds: Option<u64>) -> String {
    match seconds {
        None => "—".into(),
        Some(n) if n < 3600 => format!("{}m", n / 60),
        Some(n) if n < 86400 => format!("{:.1}h", n as f64 / 3600.0),
        Some(n) => format!("{:.1}d", n as f64 / 86400.0),
    }
}
/// When a sample was read: its time for one read today, with the date otherwise.
pub(super) fn read_stamp(read_at: u64) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let (stamp, today) = (date(read_at as i64, 0), date(now as i64, 0));
    match stamp.get(11..) {
        Some(time) if stamp.get(..10) == today.get(..10) => trf!("{time} UTC", time),
        _ => trf!("{time} UTC", time = stamp),
    }
}

/// What a frame of the tab asked the page for.
#[derive(Default)]
pub(super) struct Out {
    /// Read the list again, or load more of it.
    pub reload: bool,
    /// Open Hosting.
    pub hosting: bool,
    /// Review an export of the rows shown.
    pub export: bool,
}

pub(super) fn show(
    ui: &mut Ui,
    s: &mut State,
    widgets: &mut HashMap<String, UiControl>,
    backend: Option<&Backend>,
) -> Out {
    let (mut reload, mut hosting) = (false, false);
    if !s.loaded {
        // Nothing to filter or export yet: the page is one state.
        Frame::new()
            .inner_margin(Margin::symmetric(24, 14))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                if s.list_running() || !s.attempted {
                    pages::loading(
                        ui,
                        tr(if s.stopping {
                            mark("Stopping…")
                        } else {
                            mark("Reading pull requests…")
                        }),
                    );
                } else if s.trouble == Trouble::NoAccount {
                    hosting = no_account(ui, widgets);
                } else {
                    (reload, hosting) = trouble(ui, s, widgets);
                }
            });
        return Out { reload, hosting, export: false };
    }
    let (stamped, export) = control_row(ui, s, widgets);
    let viewport = ui.available_height();
    let scroll = ScrollArea::vertical()
        .id_salt("insights-pulls-scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let origin = ui.cursor().top();
            Frame::new()
                .inner_margin(Margin {
                    left: 24,
                    right: 24,
                    top: 4,
                    bottom: 16,
                })
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.spacing_mut().item_spacing.y = 12.0;
                    (reload, hosting) = trouble(ui, s, widgets);
                    figures(ui, s);
                    reload |= caveats(ui, s, widgets);
                    if !s.trends {
                        let (read_reviews, open_hosting) = review_strip(ui, s, widgets);
                        hosting |= open_hosting;
                        if let (true, Some(backend)) = (read_reviews, backend) {
                            s.start_reviews(backend.clone(), ui.ctx().clone());
                        }
                    }
                    let width = ui.available_width();
                    if s.trends {
                        let used = ui.cursor().top() - origin;
                        let rest = (viewport - used - 17.0).floor();
                        let asked = super::trends::show(ui, s, widgets, width >= SIDE_BY_SIDE, rest);
                        if asked.more || asked.again {
                            s.want_more = asked.more;
                            reload = true;
                        }
                        return;
                    }
                    let (chosen, retry) = if width >= SIDE_BY_SIDE {
                        // The two cards take what is left of the page, so only they
                        // scroll; a page too short for them scrolls as a whole.
                        let used = ui.cursor().top() - origin;
                        let height = (viewport - used - 17.0).max(SPLIT_MIN).floor();
                        let details = (width * 0.42).clamp(380.0, 520.0).floor();
                        let mut out = (ListOut::default(), false);
                        ui.horizontal_top(|ui| {
                            ui.spacing_mut().item_spacing.x = 16.0;
                            out.0 = list(ui, s, widgets, vec2(width - details - 16.0, height), !stamped);
                            ui.allocate_ui_with_layout(
                                vec2(details, height),
                                Layout::top_down(Align::Min),
                                |ui| {
                                    out.1 = super::pull_detail::show(
                                        ui,
                                        &mut s.detail,
                                        widgets,
                                        Some(height),
                                    );
                                },
                            );
                        });
                        out
                    } else {
                        // Stacked: a bounded list, then the details at their own height
                        // under it, so the page is the one thing that scrolls.
                        let shown = s.visible().count().clamp(2, 5) as f32;
                        let chosen =
                            list(ui, s, widgets, vec2(width, BAR + shown * ROW + 2.0), !stamped);
                        let retry = super::pull_detail::show(ui, &mut s.detail, widgets, None);
                        (chosen, retry)
                    };
                    if let Some(filter) = chosen.filter {
                        s.show = filter;
                        s.filter_changed();
                    }
                    let number = chosen
                        .selected
                        .or_else(|| if retry { s.detail.number } else { None });
                    if let (Some(number), Some(connection), Some(backend)) =
                        (number, &s.connection, backend)
                    {
                        s.detail
                            .start(number, connection.clone(), backend.clone(), ui.ctx());
                    }
                });
        });
    pages::region(
        widgets,
        "insights-scroll",
        scroll.inner_rect.intersect(ui.clip_rect()),
        true,
    );
    Out { reload, hosting, export }
}

/// No account for this repository's host: what is missing and the way to it.
/// Returns whether Open Hosting was clicked.
fn no_account(ui: &mut Ui, widgets: &mut HashMap<String, UiControl>) -> bool {
    let height = ui.available_height();
    let height = if height.is_finite() && height < 4000.0 {
        height.max(220.0)
    } else {
        260.0
    };
    let width = ui.available_width();
    ui.allocate_ui_with_layout(
        vec2(width, height),
        Layout::top_down(Align::Center),
        |ui| {
            ui.set_min_size(vec2(width, height));
            ui.spacing_mut().item_spacing.y = 8.0;
            ui.add_space(((height - 170.0) / 2.0).max(8.0));
            let (tile, _) = ui.allocate_exact_size(vec2(40.0, 40.0), Sense::hover());
            ui.painter().rect_filled(tile, 10.0, RAISED());
            icon(ui.painter(), tile.shrink(11.0), Icon::Remote, HINT());
            ui.label(RichText::new(tr("No Hosting account for this repository")).font(semibold(14.0)));
            // A block of its own width, so the sentence wraps and stays centred.
            ui.allocate_ui_with_layout(
                vec2(400.0_f32.min(width), 18.0),
                Layout::top_down(Align::Center),
                |ui| {
                    ui.add(
                        Label::new(
                            RichText::new(tr(
                                "Pull request numbers come from the repository's host. Activity and Files work without one.",
                            ))
                            .size(12.5)
                            .color(HINT()),
                        )
                        .wrap()
                        .halign(Align::Center),
                    );
                },
            );
            ui.add_space(4.0);
            let r = pages::primary(ui, true, tr("Open Hosting"));
            pages::control(widgets, "insights-hosting", &r, ui.clip_rect());
            r.clicked()
        },
    )
    .inner
}

/// A read that stopped or failed, in one line with the way on. An earlier sample
/// stays below it, and the line says when that sample is from. Returns whether to
/// read again and whether to open Hosting.
fn trouble(ui: &mut Ui, s: &State, widgets: &mut HashMap<String, UiControl>) -> (bool, bool) {
    let Some(error) = &s.error else {
        return (false, false);
    };
    let kept = if s.loaded && matches!(s.trouble, Trouble::SignIn | Trouble::Failed) {
        trf!(" The numbers below are from {time}.", time = read_stamp(s.read_at))
    } else {
        String::new()
    };
    let text = format!("{error}{kept}");
    let idle = !s.running();
    match s.trouble {
        Trouble::Stopped => {
            strip(ui, MUTED(), &text, None, widgets);
            (false, false)
        }
        Trouble::SignIn => (
            false,
            strip(ui, RED(), &text, Some(("insights-hosting", tr("Open Hosting"), true)), widgets),
        ),
        Trouble::Changed => (
            strip(ui, AMBER(), &text, Some(("insights-again", tr("Read again"), idle)), widgets),
            false,
        ),
        _ => (
            strip(ui, RED(), &text, Some(("insights-again", tr("Read again"), idle)), widgets),
            false,
        ),
    }
}

/// A tinted line across the page: a mark, its text and, at the end, one action that
/// keeps its place while it can't be used. `action` is its automation name, label
/// and whether it is enabled. Returns whether the action was clicked.
pub(super) fn strip(
    ui: &mut Ui,
    color: Color32,
    text: &str,
    action: Option<(&str, &str, bool)>,
    widgets: &mut HashMap<String, UiControl>,
) -> bool {
    strip_marked(ui, color, false, text, action, widgets)
}

/// [`strip`] whose mark is a spinner while `busy`.
fn strip_marked(
    ui: &mut Ui,
    color: Color32,
    busy: bool,
    text: &str,
    action: Option<(&str, &str, bool)>,
    widgets: &mut HashMap<String, UiControl>,
) -> bool {
    Frame::new()
        .fill(color.gamma_multiply(0.08))
        .stroke(Stroke::new(1.0, color.gamma_multiply(0.35)))
        .corner_radius(8)
        .inner_margin(Margin {
            left: 12,
            right: 8,
            top: 4,
            bottom: 4,
        })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let clicked = action.is_some_and(|(name, label, enabled)| {
                        let r = ui.add_enabled(enabled, Button::new(label));
                        pages::control(widgets, name, &r, ui.clip_rect());
                        r.clicked()
                    });
                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                        if busy {
                            ui.add(Spinner::new().size(14.0).color(color));
                        } else {
                            let (mark, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
                            icon(ui.painter(), mark, Icon::Warning, color);
                        }
                        ui.add(Label::new(RichText::new(text).size(12.5).color(color)).wrap());
                    });
                    clicked
                })
                .inner
            })
            .inner
        })
        .inner
}

/// The width of the List | Trends switch, and the page width below which it takes a
/// row of its own so the choices beside it keep their size.
const VIEW_SWITCH: f32 = 116.0;
const SWITCH_BESIDE: f32 = 640.0;

/// List | Trends.
fn view_switch(ui: &mut Ui, s: &mut State, widgets: &mut HashMap<String, UiControl>) {
    let options = [(tr("List"), !s.trends), (tr("Trends"), s.trends)];
    for (i, r) in pages::segments(ui, &options).iter().enumerate() {
        pages::control(widgets, ["insights-view-list", "insights-view-trends"][i], r, ui.clip_rect());
        if r.clicked() && s.trends != (i == 1) {
            s.trends = i == 1;
            // The reviewer narrows the list only, so the numbers change with the view.
            s.filter_changed();
        }
    }
}

/// List | Trends, period, author, reviewer, when the sample was read and Export, on one
/// row; in a narrower page the author and reviewer take a row of their own, and in a
/// narrow one the switch too. While a read runs its spinner takes the place of the time,
/// so nothing below moves. Returns whether the row had room to say when the sample was
/// read (the list says it otherwise), and whether Export was clicked.
fn control_row(
    ui: &mut Ui,
    s: &mut State,
    widgets: &mut HashMap<String, UiControl>,
) -> (bool, bool) {
    let (mut stamped, mut export) = (false, false);
    Frame::new()
        .inner_margin(Margin {
            left: 24,
            right: 24,
            top: 12,
            bottom: 8,
        })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            let width = ui.available_width();
            // In a narrow page the switch has a row of its own, above the choices.
            let beside = width >= SWITCH_BESIDE;
            if !beside {
                ui.horizontal(|ui| view_switch(ui, s, widgets));
                ui.add_space(8.0);
            }
            let switch = if beside { VIEW_SWITCH } else { 0.0 };
            // The author and reviewer share what the row leaves them, or a row of their own.
            let inline = width >= FIELDS_INLINE;
            let field = if inline {
                ((width - 520.0 - switch) / 2.0).clamp(120.0, 190.0)
            } else {
                ((width - 8.0) / 2.0).min(240.0)
            };
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                // Export keeps its place at the right; the choices take the rest.
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let r = ui.add_enabled(
                        !s.running() && s.visible().next().is_some(),
                        Button::new(tr("Export")),
                    );
                    pages::control(widgets, "insights-export", &r, ui.clip_rect());
                    export = r.clicked();
                    let reading = s.list_running();
                    let (when, read) = super::refreshed(s.read_at, ui.ctx());
                    let sample = trf!(
                        "{repo} · refreshed {when}",
                        repo = s.repository.as_deref().unwrap_or(""),
                        when
                    );
                    // The sample line shows only where the choices leave it room.
                    let fields = if inline { 2.0 * field + 8.0 } else { 0.0 };
                    let room = ui.available_width() - 250.0 - fields - 40.0 - switch;
                    let fits = |text: &str| {
                        let galley = ui.painter().layout_no_wrap(
                            text.to_owned(),
                            FontId::proportional(12.0),
                            FAINT(),
                        );
                        galley.size().x <= room
                    };
                    stamped = fits(&sample);
                    let shown = if reading {
                        tr(if s.stopping {
                            mark("Stopping…")
                        } else {
                            mark("Reading pull requests…")
                        })
                        .to_owned()
                    } else {
                        sample
                    };
                    if stamped && fits(&shown) {
                        let r = ui.label(RichText::new(shown).size(12.0).color(FAINT()));
                        if !reading {
                            r.on_hover_text(read);
                        }
                    }
                    if reading {
                        ui.add(Spinner::new().size(14.0));
                    }
                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                        if beside {
                            view_switch(ui, s, widgets);
                        }
                        if let Some(p) = periods(
                            ui,
                            s.period.unwrap_or(Period::Days30),
                            "insights-period",
                            widgets,
                        ) {
                            s.period = Some(p);
                            s.recalculate();
                        }
                        if inline {
                            author_filter(ui, s, widgets, field);
                            ui.add_enabled_ui(!s.trends, |ui| reviewer_filter(ui, s, widgets, field));
                        }
                    });
                });
            });
            if !inline {
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    author_filter(ui, s, widgets, field);
                    ui.add_enabled_ui(!s.trends, |ui| reviewer_filter(ui, s, widgets, field));
                });
            }
        });
    (stamped, export)
}

fn author_filter(
    ui: &mut Ui,
    s: &mut State,
    widgets: &mut HashMap<String, UiControl>,
    width: f32,
) {
    let previous = s.author.clone();
    let authors = s.authors();
    // The name starts after the person mark painted on the field's left edge.
    let mut name = text::LayoutJob::default();
    name.append(
        s.author.as_deref().unwrap_or(tr("All authors")),
        20.0,
        TextFormat::simple(FontId::proportional(13.0), TEXT()),
    );
    let combo = ComboBox::from_id_salt("insights-author")
        .selected_text(name)
        .width(width)
        .truncate()
        .show_ui(ui, |ui| {
            let r = ui.selectable_value(&mut s.author, None, tr("All authors"));
            pages::control(widgets, "insights-author-all", &r, ui.clip_rect());
            for (index, author) in authors.into_iter().enumerate() {
                let r = ui.selectable_value(&mut s.author, Some(author.clone()), author);
                pages::control(
                    widgets,
                    format!("insights-author-{index}"),
                    &r,
                    ui.clip_rect(),
                );
            }
        });
    let field = combo.response.rect;
    icon(
        ui.painter(),
        Rect::from_center_size(pos2(field.left() + 15.0, field.center().y), vec2(13.0, 13.0)),
        Icon::User,
        HINT(),
    );
    pages::control(widgets, "insights-author", &combo.response, ui.clip_rect());
    combo.response.on_hover_text(tr("Author"));
    if previous != s.author {
        s.filter_changed();
    }
}

/// "+9", "−3" or "±0": how a count moved since the previous period.
pub(super) fn change(current: usize, previous: usize) -> String {
    match current.cmp(&previous) {
        std::cmp::Ordering::Greater => format!("+{}", current - previous),
        std::cmp::Ordering::Less => format!("−{}", previous - current),
        std::cmp::Ordering::Equal => "±0".into(),
    }
}

/// How a median time moved, in its own units ("−0.4d"); `None` without both medians.
pub(super) fn duration_change(current: Option<u64>, previous: Option<u64>) -> Option<String> {
    let (current, previous) = (current?, previous?);
    Some(match current.cmp(&previous) {
        std::cmp::Ordering::Greater => format!("+{}", duration(Some(current - previous))),
        std::cmp::Ordering::Less => format!("−{}", duration(Some(previous - current))),
        std::cmp::Ordering::Equal => "±0".into(),
    })
}

/// The five numbers, each with what it covers: the period, the loaded sample or the
/// dated pull requests a median comes from. Opened, Merged and the merge time say how
/// they moved when the period before is loaded in full. The list adds Waiting for
/// review and Time to first review; a number that can't be worked out shows — and why.
fn figures(ui: &mut Ui, s: &State) {
    let m = &s.metrics;
    let r = &s.review_metrics;
    let chosen = s.period.unwrap_or(Period::Days30);
    let period = chosen.phrase();
    // The change, and the value it is from for its tooltip.
    let moved = |text: Option<String>, before: String| {
        text.map(|text| (text, format!("{}: {before}", chosen.previous_label())))
    };
    let (opened, merged, merge_time) = match s.comparison() {
        Comparison::Ready(p) => (
            moved(Some(change(m.opened, p.opened)), p.opened.to_string()),
            moved(Some(change(m.merged, p.merged)), p.merged.to_string()),
            moved(
                duration_change(m.median_merge_seconds, p.median_merge_seconds),
                duration(p.median_merge_seconds),
            ),
        ),
        _ => (None, None, None),
    };
    let mut figures = vec![
        (tr("Opened"), m.opened.to_string(), period.to_owned(), opened),
        (tr("Merged"), m.merged.to_string(), period.to_owned(), merged),
        (
            tr("Open now"),
            m.open_now.to_string(),
            trn!(m.loaded, "of {n} loaded", "of {n} loaded"),
            None,
        ),
        (
            tr("Median merge time"),
            duration(m.median_merge_seconds),
            // Without a median: nothing merged in the period, or merges whose
            // dates can't be used.
            if m.merge_samples > 0 {
                trn!(m.merge_samples, "from {n} merged", "from {n} merged")
            } else if m.merged == 0 {
                tr("none merged").to_owned()
            } else {
                tr("no usable dates").to_owned()
            },
            merge_time,
        ),
        (
            tr("Median open age"),
            duration(m.median_open_seconds),
            if m.open_samples > 0 {
                trn!(m.open_samples, "from {n} open", "from {n} open")
            } else if m.open_now == 0 {
                tr("none open").to_owned()
            } else {
                tr("no usable dates").to_owned()
            },
            None,
        ),
    ];
    if !s.trends {
        let waiting = if r.requests_known == 0 && r.requests_unknown > 0 {
            let note = trf!("not in {host}’s list", host = s.host());
            ("—".to_owned(), note)
        } else if r.requests_known == 0 {
            ("0".to_owned(), tr("none open").to_owned())
        } else {
            let note = match &s.reviewer {
                Some(who) => trf!("on {name}", name = who.name),
                None => trn!(r.requests, "{n} request", "{n} requests"),
            };
            (r.waiting.to_string(), note)
        };
        let first = if !s.times_reviews() {
            ("—".to_owned(), trf!("not on {host}", host = s.host()))
        } else if r.reviewed > 0 {
            (
                duration(r.median_first_review_seconds),
                trn!(r.reviewed, "from {n} reviewed", "from {n} reviewed"),
            )
        } else if s.reviews_running() {
            ("—".to_owned(), tr("reading reviews").to_owned())
        } else if r.opened == 0 {
            ("—".to_owned(), tr("none opened").to_owned())
        } else if r.read == 0 {
            ("—".to_owned(), tr("reviews not read").to_owned())
        } else if r.no_review > 0 {
            ("—".to_owned(), tr("no reviews yet").to_owned())
        } else {
            ("—".to_owned(), tr("no usable times").to_owned())
        };
        figures.push((tr("Waiting for review"), waiting.0, waiting.1, None));
        figures.push((tr("Time to first review"), first.0, first.1, None));
    }
    let width = ui.available_width();
    let rows: &[usize] = match (figures.len(), width) {
        (7, w) if w >= SEVEN_ACROSS => &[7],
        (7, w) if w >= FOUR_ACROSS => &[4, 3],
        (7, _) => &[3, 2, 2],
        (_, w) if w >= FIVE_ACROSS => &[5],
        _ => &[3, 2],
    };
    // Rows of their own sizes, eight points apart like the rows within one call.
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.y = 8.0;
        let mut at = 0;
        for &n in rows {
            figure_tiles(ui, &figures[at..at + n], n);
            at += n;
        }
    });
}

/// What limits the numbers above, in one strip: a partial sample, with the way to
/// load more, and pull requests without usable dates. Returns whether to read.
fn caveats(ui: &mut Ui, s: &mut State, widgets: &mut HashMap<String, UiControl>) -> bool {
    let undated = s.metrics.incomplete_dates;
    let r = &s.review_metrics;
    // Reviews read but none yet: the median leaves those pull requests out.
    let unreviewed = if s.trends || !s.times_reviews() { 0 } else { r.no_review };
    // Some open rows say which requests wait and some don't (a page without them).
    let unknown = if s.trends || r.requests_known == 0 { 0 } else { r.requests_unknown };
    if !s.more && !s.limited && undated == 0 && unreviewed == 0 && unknown == 0 {
        return false;
    }
    let idle = !s.running();
    let (mut text, action) = if s.more && s.pages < MAX_PAGES {
        (
            trf!(
                "Partial sample: the host has more pull requests than the {count} loaded, so these are not repository totals.",
                count = s.items.len()
            ),
            Some(("insights-more", tr("Load more"), idle)),
        )
    } else if s.more {
        (
            tr("Partial sample: this view stops at 500 pull requests a refresh, so these are not repository totals.").to_owned(),
            None,
        )
    } else if s.limited {
        (
            tr("Partial sample: the host's list was incomplete or changed while it was read, so these are not repository totals.").to_owned(),
            Some(("insights-again", tr("Read again"), idle)),
        )
    } else {
        (String::new(), None)
    };
    if undated > 0 {
        if !text.is_empty() {
            text.push(' ');
        }
        text.push_str(&trn!(
            undated,
            "{n} pull request has a missing or unusable date and is left out of timing.",
            "{n} pull requests have a missing or unusable date and are left out of timing."
        ));
    }
    let period = s.period.unwrap_or(Period::Days30).phrase();
    for sentence in [
        (unreviewed > 0).then(|| {
            trn!(
                unreviewed,
                "{n} opened {period} has no review yet, so time to first review leaves it out.",
                "{n} opened {period} have no review yet, so time to first review leaves them out.",
                period = period
            )
        }),
        (unknown > 0).then(|| {
            trn!(
                unknown,
                "{n} open pull request has no review state in the list and is left out of Waiting for review.",
                "{n} open pull requests have no review state in the list and are left out of Waiting for review."
            )
        }),
    ]
    .into_iter()
    .flatten()
    {
        if !text.is_empty() {
            text.push(' ');
        }
        text.push_str(&sentence);
    }
    let more = action.is_some_and(|a| a.0 == "insights-more");
    if !strip(ui, AMBER(), &text, action, widgets) {
        return false;
    }
    s.want_more = more;
    true
}

/// A row's state as shown and its colour: merged and open from what was read, any
/// other state in the host's word.
fn look(item: &ForgeItem) -> (String, Color32) {
    match &item.timing {
        Some(t) if t.is_merged => (tr("Merged").to_owned(), VIOLET()),
        Some(t) if t.open => (tr("Open").to_owned(), GREEN()),
        _ => crate::forge::state_look(&item.state),
    }
}

/// How long a pull request has been open, or took to merge: the short form for its
/// row and the sentence for its tooltip. `None` without the dates for it.
fn elapsed(item: &ForgeItem, now: u64) -> Option<(String, String)> {
    let t = item.timing.as_ref().filter(|t| !t.incomplete(now))?;
    let created = t.created?;
    if t.open {
        let time = duration(Some(now.checked_sub(created)?));
        Some((trf!("for {time}", time), trf!("Open for {time}", time)))
    } else if t.is_merged {
        let time = duration(Some(t.merged?.checked_sub(created)?));
        Some((trf!("in {time}", time), trf!("Merged {time} after it was opened", time)))
    } else {
        None
    }
}

/// A pull request left out of timing: the note for its row and the sentence for its
/// tooltip. A merge date alone can be unusable while the creation date still counts
/// in Opened, so that row says which date it is.
fn undated(item: &ForgeItem, now: u64) -> Option<(&'static str, &'static str)> {
    let both = (
        mark("No dates"),
        mark("Its dates are missing or not usable, so it is left out of timing."),
    );
    let Some(t) = &item.timing else {
        return Some(both);
    };
    if !t.incomplete(now) {
        None
    } else if t.created.is_some_and(|created| created <= now) {
        Some((
            mark("No merge date"),
            mark("Its merge date is missing or not usable, so it is left out of merge time."),
        ))
    } else {
        Some(both)
    }
}

/// A pill ending at `right`, centred on `y`. Returns its left edge.
fn pill_at(painter: &Painter, right: f32, y: f32, text: &str, color: Color32) -> f32 {
    let galley = painter.layout_no_wrap(text.to_owned(), semibold(11.0), color);
    let rect = Rect::from_min_max(
        pos2(right - galley.size().x - 16.0, y - 10.0),
        pos2(right, y + 10.0),
    );
    painter.rect_filled(rect, 10.0, color.gamma_multiply(0.14));
    painter.galley(pos2(rect.left() + 8.0, y - galley.size().y / 2.0), galley, color);
    rect.left()
}

#[derive(Default)]
struct ListOut {
    /// The pull request whose row was clicked.
    selected: Option<u64>,
    /// The state filter that was chosen.
    filter: Option<u8>,
}

/// The loaded pull requests as a card `size` large: its title and the state filter,
/// then rows that scroll inside it.
fn list(
    ui: &mut Ui,
    s: &State,
    widgets: &mut HashMap<String, UiControl>,
    size: Vec2,
    // Whether to say here when the sample was read: the control row had no room.
    stamp: bool,
) -> ListOut {
    let mut out = ListOut::default();
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    pages::card_back(ui, rect);
    // Inside the outline, so rows and the bar never paint over it.
    let inner = rect.shrink(1.0);
    let mut card = ui.new_child(
        UiBuilder::new()
            .max_rect(inner)
            .layout(Layout::top_down(Align::Min)),
    );
    card.set_clip_rect(inner.intersect(ui.clip_rect()));
    card.spacing_mut().item_spacing.y = 0.0;
    let counts = s.state_counts();
    pages::card_bar(&mut card, |ui| {
        let mut loaded = trn!(s.items.len(), "{n} loaded", "{n} loaded");
        if stamp {
            let (when, _) = super::refreshed(s.read_at, ui.ctx());
            loaded = format!("{loaded} · {}", trf!("refreshed {when}", when));
        }
        // The state filter keeps its place at the right; the line before it gives way.
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            // Waiting keeps its place, disabled, where no row says which requests wait.
            let waiting = s.requests_known_anywhere() || s.show == 3;
            let filters = [
                (tr("All"), Some(counts[0]), s.show == 0, true, 0u8),
                (tr("Open"), Some(counts[1]), s.show == 1, true, 1),
                (tr("Waiting"), waiting.then_some(counts[3]), s.show == 3, waiting, 3),
                (tr("Merged"), Some(counts[2]), s.show == 2, true, 2),
            ];
            let items: Vec<_> = filters.iter().map(|f| (f.0, f.1, f.2, f.3)).collect();
            for (filter, r) in filters.iter().zip(pages::tab_segments_enabled(ui, &items)) {
                let id = filter.4;
                pages::control(widgets, format!("insights-filter-{id}"), &r, ui.clip_rect());
                let r = if id != 3 {
                    r
                } else if filter.3 {
                    r.on_hover_text(tr("Open pull requests with a review request still waiting"))
                } else {
                    r.on_disabled_hover_text(trf!(
                        "{host}’s list does not say who has reviewed",
                        host = s.host()
                    ))
                };
                if r.clicked() && s.show != id {
                    out.filter = Some(id);
                }
            }
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                ui.label(RichText::new(tr("Pull requests")).font(semibold(13.0)));
                ui.add(Label::new(RichText::new(loaded).size(12.0).color(FAINT())).truncate());
            });
        });
    });
    let rows = (inner.bottom() - card.cursor().top()).max(0.0);
    if s.visible().next().is_none() {
        let words = if s.items.is_empty() {
            tr("The host returned no pull requests for this repository.")
        } else {
            tr("No loaded pull request matches. Choose another author, reviewer or state.")
        };
        card.allocate_ui_with_layout(
            vec2(inner.width(), rows),
            Layout::centered_and_justified(Direction::TopDown),
            |ui| {
                ui.add(Label::new(RichText::new(words).size(12.5).color(HINT())).wrap());
            },
        );
        return out;
    }
    let enabled = !s.list_running();
    let scroll = ScrollArea::vertical()
        .id_salt("insights-list-scroll")
        .max_height(rows)
        .auto_shrink([false, false])
        .show(&mut card, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for item in s.visible() {
                let mark = review_mark(s, item);
                let selected = s.detail.number == Some(item.number);
                let r = row(ui, item, mark, selected, enabled, s.read_at);
                pages::control(
                    widgets,
                    format!("insights-pr-{}", item.number),
                    &r,
                    ui.clip_rect(),
                );
                if r.clicked() {
                    out.selected = Some(item.number);
                }
            }
        });
    pages::region(
        widgets,
        "insights-list-scroll",
        scroll.inner_rect.intersect(ui.clip_rect()),
        true,
    );
    out
}

/// One pull request: its state mark, title and number, who opened it and its
/// branches, then how long it has been open or took to merge and its state; the
/// second line ends with what it says about its reviews.
fn row(
    ui: &mut Ui,
    item: &ForgeItem,
    review: Option<ReviewMark>,
    selected: bool,
    enabled: bool,
    now: u64,
) -> Response {
    let response = ui.add_enabled(
        enabled,
        Button::new("")
            .frame(false)
            .min_size(vec2(ui.available_width(), ROW)),
    );
    let rect = response.rect;
    let painter = ui.painter();
    if selected {
        painter.rect_filled(rect, 0.0, SELECTED());
        painter.rect_filled(
            Rect::from_min_max(
                pos2(rect.left(), rect.top() + 10.0),
                pos2(rect.left() + 3.0, rect.bottom() - 10.0),
            ),
            1.5,
            BLUE(),
        );
    } else if enabled && (response.hovered() || response.has_focus()) {
        painter.rect_filled(rect, 0.0, LIST_HOT());
    }
    painter.hline(
        rect.x_range(),
        rect.bottom() - 0.5,
        Stroke::new(1.0, CARD_LINE()),
    );
    let (word, color) = look(item);
    let mark = Rect::from_center_size(pos2(rect.left() + 27.0, rect.center().y), vec2(26.0, 26.0));
    painter.circle_filled(mark.center(), 13.0, color.gamma_multiply(0.14));
    icon(painter, mark.shrink(6.5), Icon::Merge, color);
    // From the right: the state, then the time or a note that its dates are missing.
    let mut right = rect.right() - 16.0;
    if !word.is_empty() {
        right = pill_at(painter, right, rect.center().y, &word, color) - 8.0;
    }
    let time = elapsed(item, now);
    let note = undated(item, now);
    if let Some((words, _)) = note {
        right = pill_at(painter, right, rect.center().y, tr(words), AMBER()) - 8.0;
    } else if let Some((short, _)) = &time {
        let galley = painter.layout_no_wrap(short.clone(), FontId::proportional(12.0), FAINT());
        right -= galley.size().x;
        painter.galley(pos2(right, rect.center().y - galley.size().y / 2.0), galley, FAINT());
        right -= 12.0;
    }
    let left = mark.right() + 12.0;
    let width = (right - left).max(0.0);
    let ink = if enabled { TEXT() } else { MUTED() };
    // The number stays whole after a title cut short.
    let number = painter.layout_no_wrap(format!("#{}", item.number), FontId::proportional(13.0), HINT());
    let mut title = text::LayoutJob::simple_singleline(item.title.clone(), semibold(13.0), ink);
    title.wrap = text::TextWrapping::truncate_at_width((width - number.size().x - 6.0).max(0.0));
    let title = painter.layout_job(title);
    let y = rect.center().y - 10.0;
    let end = left + title.size().x;
    painter.galley(pos2(left, y - title.size().y / 2.0), title, ink);
    if end + 6.0 + number.size().x <= right {
        painter.galley(pos2(end + 6.0, y - number.size().y / 2.0), number, HINT());
    }
    // The review mark ends the second line where the time above ends; the author and
    // branches give way to it, as long as some of them still show.
    let mut meta_width = width;
    if let Some(mark) = &review {
        let galley = painter.layout_no_wrap(mark.text.clone(), FontId::proportional(12.0), mark.color);
        let lead = if mark.icon.is_some() { 16.0 } else { 0.0 };
        let x = right - galley.size().x - lead;
        if x >= left + 80.0 {
            let y = rect.center().y + 10.0;
            if let Some(kind) = mark.icon {
                icon(painter, Rect::from_center_size(pos2(x + 6.0, y), vec2(12.0, 12.0)), kind, mark.color);
            }
            painter.galley(pos2(x + lead, y - galley.size().y / 2.0), galley, mark.color);
            meta_width = (x - 12.0 - left).max(0.0);
        }
    }
    let plain = TextFormat::simple(FontId::proportional(12.0), HINT());
    let branch = TextFormat {
        background: CHIP(),
        ..TextFormat::simple(FontId::monospace(11.5), TEXT_SOFT())
    };
    let mut meta = text::LayoutJob::default();
    meta.append(
        if item.author.is_empty() { tr("Unknown author") } else { &item.author },
        0.0,
        plain.clone(),
    );
    if !item.source.is_empty() {
        meta.append("   ", 0.0, plain.clone());
        meta.append(&format!(" {} ", item.source), 0.0, branch.clone());
        if !item.target.is_empty() {
            meta.append("  →  ", 0.0, plain);
            meta.append(&format!(" {} ", item.target), 0.0, branch);
        }
    }
    meta.wrap = text::TextWrapping::truncate_at_width(meta_width);
    let meta = painter.layout_job(meta);
    painter.galley(
        pos2(left, rect.center().y + 10.0 - meta.size().y / 2.0),
        meta,
        HINT(),
    );
    response.widget_info(|| {
        WidgetInfo::selected(
            Role::Button,
            enabled,
            selected,
            format!("#{} {}", item.number, item.title),
        )
    });
    let mut tip = if let Some((_, sentence)) = note {
        tr(sentence).to_owned()
    } else if let Some((_, sentence)) = time {
        sentence
    } else {
        String::new()
    };
    if let Some(mark) = review {
        if !tip.is_empty() {
            tip.push('\n');
        }
        tip.push_str(&mark.tip);
    }
    if tip.is_empty() {
        response
    } else {
        response.on_hover_text(tip)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    pub(super) fn item(number: u64, author: &str, open: bool) -> ForgeItem {
        ForgeItem {
            number,
            author: author.into(),
            title: format!("PR {number}"),
            state: if open { "open" } else { "closed" }.into(),
            source: "feature".into(),
            target: "main".into(),
            url: String::new(),
            reviewers: None,
            timing: Some(ForgePullTiming {
                created: Some(100),
                merged: if open { None } else { Some(150) },
                updated: None,
                open,
                is_merged: !open,
            }),
        }
    }
    #[test]
    fn authors_scope_metrics_states_only_filter_rows_and_export_freezes_review() {
        let mut s = State {
            items: vec![
                item(1, "作者", true),
                item(2, "作者", false),
                item(3, "Alice", true),
            ],
            read_at: 200,
            loaded: true,
            more: true,
            repository: Some("team/repo".into()),
            ..Default::default()
        };
        s.recalculate();
        assert_eq!(s.metrics.loaded, 3);
        assert_eq!(s.authors(), vec!["Alice", "作者"]);
        assert_eq!(s.state_counts(), [3, 2, 1, 0]);
        s.author = Some("作者".into());
        s.filter_changed();
        assert_eq!(
            (s.metrics.loaded, s.metrics.open_now, s.metrics.merged),
            (2, 1, 1)
        );
        // The filter's counts follow the author, not the state shown.
        assert_eq!(s.state_counts(), [2, 1, 1, 0]);
        s.show = 2;
        s.filter_changed();
        assert_eq!(s.visible().map(|i| i.number).collect::<Vec<_>>(), vec![2]);
        assert_eq!(s.metrics.open_now, 1);
        assert_eq!(s.state_counts(), [2, 1, 1, 0]);
        let e = super::super::export::State::new(&s, String::new());
        s.author = None;
        s.show = 0;
        s.items[1].title = "Remote changed after review".into();
        s.filter_changed();
        let snapshot = e.pulls().unwrap();
        assert_eq!(snapshot.items.len(), 1);
        assert_eq!(snapshot.items[0].title, "PR 2");
        assert_eq!(snapshot.author.as_deref(), Some("作者"));
        assert!(snapshot.partial);
    }
    #[test]
    fn filtering_hides_and_cancels_details_without_silently_resetting_missing_author() {
        let mut s = State {
            items: vec![item(1, "作者", true)],
            read_at: 200,
            ..Default::default()
        };
        s.detail.number = Some(1);
        s.author = Some("Missing".into());
        s.filter_changed();
        assert_eq!(s.metrics.loaded, 0);
        assert!(s.visible().next().is_none());
        assert_eq!(s.detail.number, None);
        assert_eq!(s.author.as_deref(), Some("Missing"));
    }
    /// The loaded page through real frames at a wide, a compact and a narrow page
    /// size: nothing runs past the page, every control keeps a usable size, and the
    /// list beside the details ends at the bottom of a wide page.
    #[test]
    fn the_page_fits_wide_compact_and_narrow_windows() {
        for (width, height, beside) in [
            // Tall enough for both strips this sample has; a shorter page scrolls.
            (1204.0, 700.0, true),
            (768.0, 498.0, false),
            // The switch, the fields' row, seven figures and two strips push a narrow
            // page's list well down.
            (560.0, 720.0, false),
        ] {
            let ctx = Context::default();
            crate::style::install(&ctx);
            let mut s = State {
                items: (1..=9)
                    .map(|n| {
                        let mut row = item(n, if n % 2 == 0 { "林 Example" } else { "Alice" }, n % 3 == 0);
                        row.title = format!("PRIVATE 改善 Unicode and a title long enough to be cut {n}");
                        row.source = "feature/a-rather-long-branch-name".into();
                        row
                    })
                    .collect(),
                read_at: 200,
                loaded: true,
                attempted: true,
                more: true,
                pages: 1,
                repository: Some("team/insights".into()),
                ..Default::default()
            };
            s.items[4].timing = None;
            // A host that times reviews, a row waiting on a long name, one read review.
            s.provider = Some(ForgeProvider::GitHub);
            s.items[2] = waiting_on(s.items[2].clone(), &["PRIVATE 林 Reviewer with a rather long name", "Bob"]);
            s.reviews.insert(2, read(Some(("Ann", 100 + 3600)), &["Ann"]));
            s.recalculate();
            let mut widgets = HashMap::new();
            let mut bounds = Rect::NOTHING;
            for _ in 0..3 {
                widgets.clear();
                let screen = Rect::from_min_size(Pos2::ZERO, vec2(width, height));
                let mut output = ctx.run_ui(
                    RawInput {
                        screen_rect: Some(screen),
                        ..Default::default()
                    },
                    |ui| {
                        show(ui, &mut s, &mut widgets, None);
                        bounds = ui.min_rect();
                    },
                );
                output.textures_delta.clear();
            }
            assert!(bounds.right() <= width + 1.0, "overflows {width}: {bounds:?}");
            for id in [
                "insights-period-30",
                "insights-period-all",
                "insights-author",
                "insights-export",
                "insights-more",
                "insights-filter-0",
                "insights-filter-2",
                "insights-filter-3",
                "insights-reviewer",
                "insights-reviews-read",
                "insights-pr-1",
            ] {
                let c = widgets.get(id).unwrap_or_else(|| panic!("{id} missing at {width}"));
                assert!(
                    c.rect.width() > 20.0 && c.rect.height() > 18.0,
                    "{id} clipped at {width}: {:?}",
                    c.rect
                );
                assert!(c.rect.right() <= width - 23.0, "{id} past the margin at {width}: {:?}", c.rect);
            }
            // The reviewer sits beside the author: on the period's row in a wide page,
            // on a row of its own below it in a narrower one.
            let (period, author, reviewer) = (
                widgets["insights-period-30"].rect,
                widgets["insights-author"].rect,
                widgets["insights-reviewer"].rect,
            );
            assert!((author.center().y - reviewer.center().y).abs() < 1.0 && reviewer.left() > author.right());
            if width - 48.0 >= FIELDS_INLINE {
                assert!((period.center().y - reviewer.center().y).abs() < 1.0, "one row at {width}");
            } else {
                assert!(reviewer.top() > period.bottom(), "a row of its own at {width}: {reviewer:?}");
            }
            let rows = widgets["insights-list-scroll"].rect;
            if beside {
                // The cards take the rest of the page, so the page itself has nothing
                // to scroll: the list ends at the bottom margin.
                assert!((rows.bottom() - (height - 17.0)).abs() <= 2.0, "list ends at {rows:?}");
                assert!(rows.width() < width * 0.6, "the details sit beside {rows:?}");
            } else {
                assert!(rows.height() <= 5.0 * ROW + 1.0, "a stacked list is bounded: {rows:?}");
                assert!(rows.width() > width - 60.0);
            }
        }
    }
    /// The states before a sample: an account is asked for in the middle of the
    /// page, and a failed first read offers to read again.
    #[test]
    fn the_page_before_a_sample_asks_for_an_account_or_offers_to_read_again() {
        let ctx = Context::default();
        crate::style::install(&ctx);
        let frame = |s: &mut State| {
            let mut widgets = HashMap::new();
            for _ in 0..3 {
                widgets.clear();
                let screen = Rect::from_min_size(Pos2::ZERO, vec2(900.0, 500.0));
                let mut output = ctx.run_ui(
                    RawInput {
                        screen_rect: Some(screen),
                        ..Default::default()
                    },
                    |ui| {
                        show(ui, s, &mut widgets, None);
                    },
                );
                output.textures_delta.clear();
            }
            widgets
        };
        let mut s = State {
            attempted: true,
            ..Default::default()
        };
        s.fail(Trouble::NoAccount, "Connect a Hosting account".into());
        let widgets = frame(&mut s);
        let open = widgets["insights-hosting"].rect;
        assert!((open.center().x - 450.0).abs() < 2.0, "centred: {open:?}");
        assert!(!widgets.contains_key("insights-export"));
        s.fail(Trouble::Failed, "The host could not be reached.".into());
        let widgets = frame(&mut s);
        assert!(widgets["insights-again"].enabled);
        assert!(!widgets.contains_key("insights-hosting"));
        s.fail(Trouble::SignIn, "The sign-in was refused.".into());
        assert!(frame(&mut s).contains_key("insights-hosting"));
    }
    #[test]
    fn rows_say_their_state_and_time_and_mark_unusable_dates() {
        let open = item(1, "A", true);
        let merged = item(2, "A", false);
        assert_eq!(look(&open).0, "Open");
        assert_eq!(look(&merged).0, "Merged");
        // Closed without a merge keeps the host's word.
        let mut closed = item(3, "A", false);
        closed.timing = Some(ForgePullTiming {
            created: Some(100),
            ..Default::default()
        });
        assert_eq!(look(&closed).0, "Closed");
        assert_eq!(elapsed(&closed, 200), None);
        assert_eq!(elapsed(&open, 100 + 3 * 86400).unwrap().0, "for 3.0d");
        assert_eq!(elapsed(&merged, 200).unwrap().0, "in 0m");
        // A merge before its creation, a future creation and no dates at all have no
        // time, so their rows get the note instead.
        let mut backwards = item(4, "A", false);
        backwards.timing.as_mut().unwrap().merged = Some(50);
        let mut future = item(5, "A", true);
        future.timing.as_mut().unwrap().created = Some(900);
        let mut none = item(6, "A", true);
        none.timing = None;
        for (row, note) in [
            (&backwards, "No merge date"),
            (&future, "No dates"),
            (&none, "No dates"),
        ] {
            assert_eq!(elapsed(row, 200), None);
            assert_eq!(undated(row, 200).map(|n| n.0), Some(note));
        }
        assert_eq!(undated(&open, 200), None);
        assert_eq!(undated(&merged, 200), None);
    }
    const DAY: u64 = 86_400;
    pub(super) fn person(name: &str) -> ForgeReviewer {
        ForgeReviewer { team: false, name: name.into() }
    }
    pub(super) fn waiting_on(mut row: ForgeItem, names: &[&str]) -> ForgeItem {
        row.reviewers = Some(ForgeReviewRequests {
            known: true,
            waiting: names.iter().map(|n| person(n)).collect(),
            ..Default::default()
        });
        row
    }
    pub(super) fn read(first: Option<(&str, u64)>, reviewers: &[&str]) -> ForgeReviewRead {
        ForgeReviewRead::Read(ForgeFirstReview {
            first: first.map(|(who, at)| (who.into(), at)),
            reviewers: reviewers.iter().map(|r| r.to_string()).collect(),
            ..Default::default()
        })
    }
    fn dated(number: u64, created: u64, merged: Option<u64>, updated: u64) -> ForgeItem {
        let mut row = item(number, "A", merged.is_none());
        row.timing = Some(ForgePullTiming {
            created: Some(created),
            merged,
            updated: Some(updated),
            open: merged.is_none(),
            is_merged: merged.is_some(),
        });
        row
    }
    /// 400 days in: two opened and one merged in the last 30 days, three opened and
    /// one merged in the 30 before, the oldest update 45 days ago.
    fn periods_sample() -> State {
        let now = 400 * DAY;
        let mut s = State {
            items: vec![
                dated(1, now - 5 * DAY, Some(now - 4 * DAY), now - 4 * DAY),
                dated(2, now - 10 * DAY, None, now - DAY),
                dated(3, now - 40 * DAY, Some(now - 35 * DAY), now - 35 * DAY),
                dated(4, now - 50 * DAY, None, now - 45 * DAY),
                dated(5, now - 45 * DAY, None, now - 44 * DAY),
            ],
            read_at: now,
            loaded: true,
            attempted: true,
            pages: 1,
            provider: Some(ForgeProvider::GitHub),
            repository: Some("team/insights".into()),
            ..Default::default()
        };
        s.recalculate();
        s
    }
    #[test]
    fn periods_compare_only_when_both_are_loaded_and_tiles_say_the_change() {
        let mut s = periods_sample();
        let Comparison::Ready(p) = s.comparison() else { panic!("a complete sample compares") };
        assert_eq!((p.opened, p.merged, p.merge_samples), (3, 1, 1));
        assert_eq!((s.metrics.opened, s.metrics.merged), (2, 1));
        assert_eq!(s.previous_range(), Some((340 * DAY, 370 * DAY)));
        // The trend's bars add up to the tiles.
        let sum = |f: fn(&ForgeTrendBucket) -> usize| s.trend.buckets.iter().map(f).sum::<usize>();
        assert_eq!((sum(|b| b.opened), sum(|b| b.merged)), (2, 1));
        assert_eq!(s.trend.unit, ForgeTrendUnit::Week);
        // More pages: the sample reaches back to its oldest update, 45 days ago, short
        // of the previous 30 days' start 60 days ago.
        s.more = true;
        s.recalculate();
        assert_eq!(s.coverage, ForgePullCoverage::After(355 * DAY));
        assert!(matches!(s.comparison(), Comparison::NotLoaded(Some(t)) if t == 355 * DAY));
        // A host whose list isn't sorted by update can't say how far back it reaches.
        s.provider = Some(ForgeProvider::Azure);
        s.recalculate();
        assert!(matches!(s.comparison(), Comparison::NotLoaded(None)));
        // A list that changed while it was read is never compared.
        s.provider = Some(ForgeProvider::GitHub);
        s.more = false;
        s.limited = true;
        s.recalculate();
        assert!(matches!(s.comparison(), Comparison::Changed));
        // All history has nothing before it.
        s.limited = false;
        s.period = Some(Period::All);
        s.recalculate();
        assert!(matches!(s.comparison(), Comparison::None));
        assert_eq!(s.previous, None);
        // The author filter applies to the trend and the period before as to the tiles.
        s.period = Some(Period::Days30);
        s.author = Some("Nobody".into());
        s.filter_changed();
        let Comparison::Ready(p) = s.comparison() else { panic!() };
        assert_eq!((p.opened, s.metrics.opened, s.trend.buckets.iter().map(|b| b.opened).sum::<usize>()), (0, 0, 0));
        // How changes read.
        assert_eq!([change(5, 3), change(3, 5), change(2, 2)], ["+2", "−2", "±0"]);
        assert_eq!(duration_change(Some(30 * 3600), Some(40 * 3600)).as_deref(), Some("−10.0h"));
        assert_eq!(duration_change(Some(2 * DAY), Some(DAY)).as_deref(), Some("+1.0d"));
        assert_eq!(duration_change(None, Some(1)), None);
    }
    #[test]
    fn leaving_during_the_first_read_reads_again_when_shown() {
        let hold = || {
            ReadTask::<ForgePage>::spawn(|cancel| loop {
                cancel.check()?;
                std::thread::sleep(std::time::Duration::from_millis(5));
            })
            .unwrap()
        };
        let mut s = State {
            attempted: true,
            ..Default::default()
        };
        s.task = Some(hold());
        s.park();
        // Nothing loaded: the next visit reads, rather than showing an empty tab.
        assert!(s.task.is_none() && !s.attempted && !s.loaded && s.error.is_none());
        // A loaded sample stays, and the next visit shows it.
        let mut s = periods_sample();
        s.task = Some(hold());
        s.park();
        assert!(s.task.is_none() && s.attempted && s.loaded);
    }
    /// Trends through real frames at a wide, a compact and a narrow page: nothing runs
    /// past the page, the switch, chart and comparison are there, and a wide chart
    /// fills the page.
    #[test]
    fn the_trends_view_fits_wide_compact_and_narrow_windows() {
        // Stacked pages are tall enough to show the comparison without scrolling.
        for (width, height, wide) in [(1204.0, 640.0, true), (768.0, 900.0, false), (560.0, 900.0, false)] {
            let ctx = Context::default();
            crate::style::install(&ctx);
            let mut s = periods_sample();
            s.trends = true;
            s.more = true;
            s.recalculate();
            let mut widgets = HashMap::new();
            let mut bounds = Rect::NOTHING;
            for _ in 0..3 {
                widgets.clear();
                let screen = Rect::from_min_size(Pos2::ZERO, vec2(width, height));
                let mut output = ctx.run_ui(
                    RawInput {
                        screen_rect: Some(screen),
                        ..Default::default()
                    },
                    |ui| {
                        show(ui, &mut s, &mut widgets, None);
                        bounds = ui.min_rect();
                    },
                );
                output.textures_delta.clear();
            }
            assert!(bounds.right() <= width + 1.0, "overflows {width}: {bounds:?}");
            for id in [
                "insights-view-list",
                "insights-view-trends",
                "insights-period-30",
                "insights-author",
                "insights-export",
                "insights-trend-chart",
                "insights-compare-more",
            ] {
                let c = widgets.get(id).unwrap_or_else(|| panic!("{id} missing at {width}"));
                assert!(c.rect.width() > 20.0 && c.rect.height() > 18.0, "{id} clipped at {width}: {:?}", c.rect);
                assert!(c.rect.right() <= width - 23.0, "{id} past the margin at {width}: {:?}", c.rect);
            }
            // The list and its rows aren't drawn in Trends.
            assert!(!widgets.contains_key("insights-pr-1"));
            let chart = widgets["insights-trend-chart"].rect;
            if wide {
                // The chart's card takes the rest of the page, beside the comparison.
                assert!((chart.bottom() - (height - 17.0 - 14.0)).abs() <= 3.0, "chart ends at {chart:?}");
                assert!(widgets["insights-compare-more"].rect.left() > chart.right(), "the comparison sits beside");
            } else {
                assert!(widgets["insights-compare-more"].rect.top() > chart.bottom(), "the comparison sits under");
            }
        }
    }
}
