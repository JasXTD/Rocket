//! Read-only PR metrics over an explicitly bounded, deduplicated list snapshot.
//! Never infer a merge time from updated_at: comments can change it after merging.
use super::*;
use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ForgePullTiming {
    pub created: Option<u64>,
    pub merged: Option<u64>,
    /// When the host last changed the pull request. Lists are read newest-updated
    /// first, so the oldest update loaded says how far back a partial sample is
    /// complete ([`ForgePullCoverage`]). Never a merge time.
    pub updated: Option<u64>,
    pub open: bool,
    pub is_merged: bool,
}
impl ForgePullTiming {
    /// Whether a date this pull request needs is missing, in the future or, for a
    /// merge, before its creation. Such a row is counted in
    /// [`ForgePullMetrics::incomplete_dates`] and left out of the timing it lacks.
    pub fn incomplete(&self, now: u64) -> bool {
        self.created.is_none_or(|c| c > now) || (self.is_merged && self.merge_time(now).is_none())
    }
    /// The merge time of a merged pull request when it can be used: not in the future
    /// and not before its creation.
    pub fn merge_time(&self, now: u64) -> Option<u64> {
        self.merged
            .filter(|m| self.is_merged && *m <= now && self.created.is_none_or(|c| *m >= c))
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ForgePullMetrics {
    pub loaded: usize,
    pub opened: usize,
    /// Merged during the selected period, with a known valid merge timestamp.
    pub merged: usize,
    /// Open across the whole loaded snapshot, independent of the period.
    pub open_now: usize,
    pub median_merge_seconds: Option<u64>,
    pub merge_samples: usize,
    pub median_open_seconds: Option<u64>,
    pub open_samples: usize,
    /// Rows missing dates needed for their current state, including invalid/future dates.
    pub incomplete_dates: usize,
}
impl ForgePullMetrics {
    pub fn from_items(items: &[ForgeItem], since: Option<u64>, now: u64) -> Self {
        let mut out = Self::default();
        let (mut merge_times, mut open_ages, mut seen) = (Vec::new(), Vec::new(), HashSet::new());
        let in_period = |time: u64| time <= now && since.is_none_or(|start| time >= start);
        for item in items {
            if !seen.insert(item.number) {
                continue;
            }
            out.loaded += 1;
            let Some(t) = &item.timing else {
                out.incomplete_dates += 1;
                continue;
            };
            let created = t.created.filter(|n| *n <= now);
            if created.is_some_and(in_period) {
                out.opened += 1;
            }
            if t.open {
                out.open_now += 1;
                if let Some(created) = created {
                    open_ages.push(now - created);
                }
            }
            if let Some(merged) = t.merge_time(now).filter(|m| in_period(*m)) {
                out.merged += 1;
                if let Some(created) = created {
                    merge_times.push(merged - created);
                }
            }
            out.incomplete_dates += usize::from(t.incomplete(now));
        }
        out.merge_samples = merge_times.len();
        out.open_samples = open_ages.len();
        out.median_merge_seconds = median(&mut merge_times);
        out.median_open_seconds = median(&mut open_ages);
        out
    }
}
fn median(values: &mut [u64]) -> Option<u64> {
    if values.is_empty() {
        return None;
    }
    values.sort_unstable();
    let high = values.len() / 2;
    Some(if values.len() % 2 == 1 {
        values[high]
    } else {
        values[high - 1] + (values[high] - values[high - 1]) / 2
    })
}

/// Whether a host's pull request list is read newest-updated first: the list paths in
/// adapter.rs sort GitHub, GitLab, Gitea and Bitbucket Cloud by update time. Azure
/// DevOps and Bitbucket Server don't promise an order, so a partial sample of theirs
/// can't say how far back it is complete.
pub fn list_sorted_by_update(provider: ForgeProvider) -> bool {
    matches!(
        provider,
        ForgeProvider::GitHub
            | ForgeProvider::GitHubEnterprise
            | ForgeProvider::GitLab
            | ForgeProvider::Gitea
            | ForgeProvider::Bitbucket
    )
}

/// How far back a loaded sample holds every pull request.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ForgePullCoverage {
    /// The host listed no further page.
    Complete,
    /// Every pull request opened or merged after this time (Unix seconds) is loaded: a
    /// list read newest-updated first stops at its oldest update, and opening or
    /// merging a pull request updates it.
    After(u64),
    /// More exist, and nothing says which periods the sample holds in full.
    #[default]
    Unknown,
}
impl ForgePullCoverage {
    /// The coverage of `items`, as read from `provider`'s list; `more` when the host
    /// has further pages.
    pub fn of(items: &[ForgeItem], provider: ForgeProvider, more: bool) -> Self {
        if !more {
            return Self::Complete;
        }
        if !list_sorted_by_update(provider) {
            return Self::Unknown;
        }
        // A row without an update time could have come from anywhere in the list.
        items
            .iter()
            .map(|i| i.timing.as_ref().and_then(|t| t.updated))
            .try_fold(None, |oldest: Option<u64>, updated| {
                updated.map(|u| Some(oldest.map_or(u, |o| o.min(u))))
            })
            .flatten()
            .map_or(Self::Unknown, Self::After)
    }
    /// Whether every pull request opened or merged at or after `time` is loaded. Rows
    /// updated exactly at the boundary may continue on the next page, so it isn't.
    pub fn covers(self, time: u64) -> bool {
        match self {
            Self::Complete => true,
            Self::After(oldest) => time > oldest,
            Self::Unknown => false,
        }
    }
}

/// What was opened and merged in one period, for comparing two periods.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ForgePullCounts {
    pub opened: usize,
    /// With a usable merge date in the period.
    pub merged: usize,
    pub median_merge_seconds: Option<u64>,
    /// Merges in the period whose creation date is usable too.
    pub merge_samples: usize,
}
impl ForgePullCounts {
    /// Pull requests opened, and merged, at or after `start` and before `end` (Unix
    /// seconds), never after `now`, each number once, by the same date rules as
    /// [`ForgePullMetrics::from_items`].
    pub fn between(items: &[ForgeItem], start: u64, end: u64, now: u64) -> Self {
        let mut out = Self::default();
        let (mut merge_times, mut seen) = (Vec::new(), HashSet::new());
        let in_period = |time: u64| time >= start && time < end && time <= now;
        for t in items
            .iter()
            .filter(|i| seen.insert(i.number))
            .filter_map(|i| i.timing.as_ref())
        {
            let created = t.created.filter(|c| *c <= now);
            out.opened += usize::from(created.is_some_and(in_period));
            if let Some(merged) = t.merge_time(now).filter(|m| in_period(*m)) {
                out.merged += 1;
                if let Some(created) = created {
                    merge_times.push(merged - created);
                }
            }
        }
        out.merge_samples = merge_times.len();
        out.median_merge_seconds = median(&mut merge_times);
        out
    }
}

/// How long one bucket of a trend is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ForgeTrendUnit {
    #[default]
    Week,
    Month,
    Year,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ForgeTrendBucket {
    /// Where it starts (Unix seconds, UTC): a Monday, the first of a month or of a
    /// year, or the period's own start for the first bucket.
    pub start: u64,
    pub opened: usize,
    pub merged: usize,
}

/// Pull requests opened and merged over a period, oldest bucket first, empty ones kept.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ForgePullTrend {
    pub unit: ForgeTrendUnit,
    pub buckets: Vec<ForgeTrendBucket>,
}
impl ForgePullTrend {
    /// Weeks (from Monday, UTC) from `since` to `now`, or from the first opening or
    /// merge when `since` is `None`; by month once that spans 53 weeks or more, and
    /// by year past 48 months. The buckets add up to [`ForgePullMetrics::from_items`]'s
    /// opened and merged for the same period.
    pub fn from_items(items: &[ForgeItem], since: Option<u64>, now: u64) -> Self {
        let in_period = |time: u64| time <= now && since.is_none_or(|start| time >= start);
        let (mut events, mut seen) = (Vec::new(), HashSet::new());
        for t in items
            .iter()
            .filter(|i| seen.insert(i.number))
            .filter_map(|i| i.timing.as_ref())
        {
            if let Some(created) = t.created.filter(|c| in_period(*c)) {
                events.push((created, false));
            }
            if let Some(merged) = t.merge_time(now).filter(|m| in_period(*m)) {
                events.push((merged, true));
            }
        }
        let Some(start) = since.or_else(|| events.iter().map(|e| e.0).min()) else {
            return Self::default();
        };
        let start = start.min(now);
        let (first, last) = ((start / 86_400) as i64, (now / 86_400) as i64);
        // Day numbers where each bucket starts; 1970-01-01 was a Thursday.
        let monday = |day: i64| day - (day + 3).rem_euclid(7);
        let (unit, days): (_, Vec<i64>) = if now - start < 53 * 7 * 86_400 {
            let weeks = (monday(first)..=monday(last)).step_by(7).collect();
            (ForgeTrendUnit::Week, weeks)
        } else {
            let ((y0, m0), (y1, m1)) = (
                crate::statistics::year_month(first),
                crate::statistics::year_month(last),
            );
            let months = (y1 - y0) as i64 * 12 + i64::from(m1) - i64::from(m0) + 1;
            if months <= 48 {
                let starts = (0..months)
                    .map(|i| {
                        let month = i64::from(m0) - 1 + i;
                        let year = y0 + month.div_euclid(12) as i32;
                        crate::statistics::days_from_civil(year, month.rem_euclid(12) as u32 + 1, 1)
                    })
                    .collect();
                (ForgeTrendUnit::Month, starts)
            } else {
                let starts = (y0..=y1)
                    .map(|year| crate::statistics::days_from_civil(year, 1, 1))
                    .collect();
                (ForgeTrendUnit::Year, starts)
            }
        };
        let mut buckets: Vec<ForgeTrendBucket> = days
            .iter()
            .map(|day| ForgeTrendBucket {
                start: (day * 86_400).max(start as i64) as u64,
                ..Default::default()
            })
            .collect();
        for (time, merged) in events {
            // The last bucket starting at or before the event; the first starts on or
            // before `start`, which no event precedes.
            let day = (time / 86_400) as i64;
            let Some(i) = days.partition_point(|d| *d <= day).checked_sub(1) else {
                continue;
            };
            if merged {
                buckets[i].merged += 1;
            } else {
                buckets[i].opened += 1;
            }
        }
        Self { unit, buckets }
    }
}

// Validate calendar and zone fields before the shared conversion. Malformed dates
// are missing evidence, never zero-duration PRs or normalized into another day.
fn date(s: &str) -> Option<u64> {
    if !s.is_ascii() || s.len() < 20 {
        return None;
    }
    let number = |r: std::ops::Range<usize>| s.get(r)?.parse::<u32>().ok();
    let (year, month, day) = (number(0..4)?, number(5..7)?, number(8..10)?);
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days = match month {
        2 => {
            if leap {
                29
            } else {
                28
            }
        }
        4 | 6 | 9 | 11 => 30,
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        _ => return None,
    };
    if day == 0 || day > days {
        return None;
    }
    let rest = s.get(19..)?;
    let zone = if let Some(frac) = rest.strip_prefix('.') {
        let n = frac.bytes().take_while(u8::is_ascii_digit).count();
        if n == 0 {
            return None;
        }
        &frac[n..]
    } else {
        rest
    };
    if zone != "Z" && zone != "z" {
        if zone.len() != 6
            || !matches!(zone.as_bytes()[0], b'+' | b'-')
            || zone.as_bytes()[3] != b':'
            || !zone[1..3]
                .bytes()
                .chain(zone[4..6].bytes())
                .all(|b| b.is_ascii_digit())
            || zone[1..3].parse::<u32>().ok()? > 23
            || zone[4..6].parse::<u32>().ok()? > 59
        {
            return None;
        }
    }
    checks::timestamp(s)
}
pub(super) fn timing(provider: ForgeProvider, v: &Json) -> ForgePullTiming {
    let state = v.str("state").or_else(|| v.str("status")).unwrap_or("");
    let created_key = match provider {
        ForgeProvider::Bitbucket => "created_on",
        ForgeProvider::Azure | ForgeProvider::AzureServer => "creationDate",
        _ => "created_at",
    };
    let is_merged = match provider {
        ForgeProvider::GitHub | ForgeProvider::GitHubEnterprise | ForgeProvider::Gitea => {
            v.str("merged_at").is_some_and(|s| !s.is_empty())
                || v.get("merged") == Some(&Json::Bool(true))
        }
        ForgeProvider::Azure | ForgeProvider::AzureServer => state == "completed",
        _ => state.eq_ignore_ascii_case("merged"),
    };
    let merged = if is_merged {
        match provider {
            // Cloud lists expose updated_on, not a trustworthy merge event time.
            ForgeProvider::Bitbucket => None,
            ForgeProvider::BitbucketServer => v.u64("closedDate").map(|n| n / 1000),
            ForgeProvider::Azure | ForgeProvider::AzureServer => v.str("closedDate").and_then(date),
            _ => v.str("merged_at").and_then(date),
        }
    } else {
        None
    };
    ForgePullTiming {
        created: if provider == ForgeProvider::BitbucketServer {
            v.u64("createdDate").map(|n| n / 1000)
        } else {
            v.str(created_key).and_then(date)
        },
        merged,
        // Azure DevOps lists have no update time.
        updated: match provider {
            ForgeProvider::Bitbucket => v.str("updated_on").and_then(date),
            ForgeProvider::BitbucketServer => v.u64("updatedDate").map(|n| n / 1000),
            ForgeProvider::Azure | ForgeProvider::AzureServer => None,
            _ => v.str("updated_at").and_then(date),
        },
        is_merged,
        open: matches!(
            state.to_ascii_lowercase().as_str(),
            "open" | "opened" | "locked" | "active"
        ),
    }
}

/// A person or team asked to review a pull request, or who reviewed it.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ForgeReviewer {
    /// Teams sort after people.
    pub team: bool,
    /// The same kind of name as an author on this host (a login, a username or a
    /// display name). A GitHub or Gitea team is `owner/team`.
    pub name: String,
}

/// What a host's pull request list says about one pull request's reviewers.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ForgeReviewRequests {
    /// The list says which requests still wait for a review. False on GitLab, whose
    /// list has no review state, and for a row whose reviewers were malformed or too
    /// many: then an empty `waiting` proves nothing.
    pub known: bool,
    /// Requests still waiting for a review, in the host's order.
    pub waiting: Vec<ForgeReviewer>,
    /// Reviewers the list shows with a verdict: an Azure DevOps vote, or a Bitbucket
    /// approval, Needs work or request for changes.
    pub reviewed: Vec<ForgeReviewer>,
    /// Reviewers listed without a review state (GitLab).
    pub assigned: Vec<ForgeReviewer>,
}
impl ForgeReviewRequests {
    /// Whether the row names `reviewer` in any role.
    pub fn names(&self, reviewer: &ForgeReviewer) -> bool {
        self.waiting
            .iter()
            .chain(&self.reviewed)
            .chain(&self.assigned)
            .any(|r| r == reviewer)
    }
}

/// More reviewers on one row than this leaves the row's review state unknown.
const MAX_REVIEWERS: usize = 100;

/// Adds one reviewer by name; false for a missing or unusable name.
fn put(list: &mut Vec<ForgeReviewer>, name: Option<&str>, team: bool) -> bool {
    let Some(name) = name.map(str::trim).filter(|n| !n.is_empty() && n.len() <= 1024) else {
        return false;
    };
    let reviewer = ForgeReviewer {
        team,
        name: crate::display_text::bounded(name, 256).text,
    };
    if !list.contains(&reviewer) {
        list.push(reviewer);
    }
    true
}

/// A list field holding reviewers: `None` when the row lacks it, `Some(None)` when it
/// is not a list. JSON null is an empty list (Gitea writes empty lists as null).
fn reviewer_list<'a>(v: &'a Json, key: &str) -> Option<Option<&'a [Json]>> {
    match v.get(key)? {
        Json::Null => Some(Some(&[])),
        list => Some(list.array()),
    }
}

/// At most [`MAX_REVIEWERS`] of a list; a longer list, or one that is not a list,
/// leaves the row's state unknown.
fn bounded<'a>(list: Option<&'a [Json]>, known: &mut bool) -> &'a [Json] {
    match list {
        Some(list) => {
            *known &= list.len() <= MAX_REVIEWERS;
            &list[..list.len().min(MAX_REVIEWERS)]
        }
        None => {
            *known = false;
            &[]
        }
    }
}

/// A list row's reviewers. `None` when the row has no reviewer fields at all: a
/// search result, or Bitbucket Cloud when the read did not ask for them.
pub(super) fn review_requests(
    provider: ForgeProvider,
    owner: &str,
    v: &Json,
) -> Option<ForgeReviewRequests> {
    let (mut waiting, mut reviewed, mut assigned) = (Vec::new(), Vec::new(), Vec::new());
    let mut known = true;
    match provider {
        ForgeProvider::GitHub | ForgeProvider::GitHubEnterprise => {
            // A requested reviewer leaves the list once they review.
            let users = reviewer_list(v, "requested_reviewers")?;
            let teams = reviewer_list(v, "requested_teams").unwrap_or(Some(&[][..]));
            for user in bounded(users, &mut known) {
                known &= put(&mut waiting, user.str("login"), false);
            }
            for team in bounded(teams, &mut known) {
                let name = team.str("slug").map(|slug| format!("{owner}/{slug}"));
                known &= put(&mut waiting, name.as_deref(), true);
            }
        }
        ForgeProvider::Gitea => {
            // Gitea's list fills `requested_reviewers` with everyone whose latest review
            // is a request, an approval or a rejection, and leaves the teams out, so it
            // names reviewers without saying who still waits.
            let users = reviewer_list(v, "requested_reviewers")?;
            let mut unused = true;
            for user in bounded(users, &mut unused) {
                put(&mut assigned, user.str("login"), false);
            }
            known = false;
        }
        ForgeProvider::GitLab => {
            // `reviewers` names who was asked; its `state` is the account's, not a review.
            let users = reviewer_list(v, "reviewers")?;
            let mut unused = true;
            for user in bounded(users, &mut unused) {
                put(&mut assigned, user.str("username"), false);
            }
            known = false;
        }
        ForgeProvider::Bitbucket => {
            let (listed, participants) = (reviewer_list(v, "reviewers"), reviewer_list(v, "participants"));
            if listed.is_none() && participants.is_none() {
                return None;
            }
            let listed = bounded(listed.unwrap_or(Some(&[][..])), &mut known);
            let participants = bounded(participants.unwrap_or(Some(&[][..])), &mut known);
            let verdict = |p: &Json| {
                p.bool("approved") == Some(true)
                    || matches!(p.str("state"), Some("approved" | "changes_requested"))
            };
            let same = |a: &Json, b: &Json| match (a.str("uuid"), b.str("uuid")) {
                (Some(a), Some(b)) => a == b,
                _ => a.str("display_name").is_some() && a.str("display_name") == b.str("display_name"),
            };
            for reviewer in listed {
                let decided = participants
                    .iter()
                    .any(|p| verdict(p) && p.get("user").is_some_and(|u| same(u, reviewer)));
                let list = if decided { &mut reviewed } else { &mut waiting };
                known &= put(list, reviewer.str("display_name"), false);
            }
            // Someone who approved without being asked still reviewed.
            for participant in participants.iter().filter(|p| verdict(p)) {
                known &= put(
                    &mut reviewed,
                    participant.get("user").and_then(|u| u.str("display_name")),
                    false,
                );
            }
        }
        ForgeProvider::BitbucketServer => {
            let listed = reviewer_list(v, "reviewers")?;
            for reviewer in bounded(listed, &mut known) {
                let user = reviewer.get("user");
                let name = user.and_then(|u| u.str("displayName").or_else(|| u.str("name")));
                known &= match reviewer.str("status") {
                    Some("UNAPPROVED") => put(&mut waiting, name, false),
                    Some("APPROVED" | "NEEDS_WORK") => put(&mut reviewed, name, false),
                    _ => false,
                };
            }
        }
        ForgeProvider::Azure | ForgeProvider::AzureServer => {
            let listed = reviewer_list(v, "reviewers")?;
            for reviewer in bounded(listed, &mut known) {
                let (name, team) = (reviewer.str("displayName"), reviewer.bool("isContainer") == Some(true));
                known &= match reviewer.get("vote") {
                    // Declining is neither a pending request nor a review.
                    Some(Json::Number(n)) if n == "0" => {
                        reviewer.bool("hasDeclined") == Some(true) || put(&mut waiting, name, team)
                    }
                    Some(Json::Number(n)) if matches!(n.as_str(), "10" | "5" | "-5" | "-10") => {
                        put(&mut reviewed, name, team)
                    }
                    _ => false,
                };
            }
        }
    }
    // A reviewer who already reviewed is no longer waiting, whatever the order.
    waiting.retain(|r| !reviewed.contains(r));
    Some(ForgeReviewRequests {
        known,
        waiting,
        reviewed,
        assigned,
    })
}

impl ForgeProvider {
    /// Whether Read reviews can time a first review on this host: its reviews carry
    /// the time each was submitted.
    pub fn times_reviews(self) -> bool {
        self.is_github() || self == Self::Gitea
    }
}

/// One pull request's reviews as read: who reviewed it, and its first review by
/// someone other than its author.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ForgeFirstReview {
    /// The earliest submitted review by someone other than the author, not a bot,
    /// with a usable time: who, and when (Unix seconds).
    pub first: Option<(String, u64)>,
    /// Everyone other than the author who submitted a review, bots too, in the
    /// order read.
    pub reviewers: Vec<String>,
    /// A review that would count has a missing or unusable time, so the first
    /// review cannot be told.
    pub undated: bool,
    /// The host has more reviews than were read and the first review cannot be
    /// told from those: Gitea documents no order, and on GitHub none of the reviews
    /// read counted.
    pub limited: bool,
}

/// What reading one pull request's reviews gave.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ForgeReviewRead {
    Read(ForgeFirstReview),
    Failed(ForgeFailure),
}

/// A bot account: GitHub marks apps with type Bot and a `[bot]` login suffix.
fn bot(user: &Json, login: &str) -> bool {
    user.str("type") == Some("Bot") || login.to_ascii_lowercase().ends_with("[bot]")
}

impl ForgeSession {
    /// Reads one pull request's submitted reviews to find its first review by
    /// someone other than its author. GitHub returns reviews oldest first, so its
    /// pages are read, up to three of 100, only until a review counts; Gitea documents
    /// no order, so two pages of 50 are read. More reviews than that, with the first
    /// still untold, mark the read `limited`. Bots are left out of `first`. `now` is
    /// the latest time a review may carry; a later one counts only as unusable, and
    /// only when no usable review is found.
    pub fn first_review(
        &self,
        repo: &ForgeRepository,
        item: &ForgeItem,
        now: u64,
        cancel: &CancellationToken,
    ) -> Result<ForgeFirstReview> {
        self.check_repository(repo)?;
        let provider = self.config.provider();
        if !provider.times_reviews() {
            return fail(ForgeFailure::Unsupported);
        }
        if item.number == 0 {
            return Err(invalid("Select a valid pull request."));
        }
        let created = item.timing.as_ref().and_then(|t| t.created).filter(|c| *c <= now);
        let gitea = provider == ForgeProvider::Gitea;
        let (size, pages) = if gitea { (50, 2) } else { (100, 3) };
        let (mut out, mut later) = (ForgeFirstReview::default(), false);
        for page in 1..=pages {
            cancel.check()?;
            let path = format!(
                "{}/pulls/{}/reviews?{}={size}&page={page}",
                self.prefix(repo),
                item.number,
                if gitea { "limit" } else { "per_page" }
            );
            let (value, headers) = self.request(
                "insights-reviews",
                "read_pull_reviews",
                "GET",
                &path,
                None,
                false,
                cancel,
            )?;
            let rows = value
                .array()
                .filter(|rows| rows.len() <= size)
                .ok_or(Error::Forge(ForgeFailure::InvalidResponse))?;
            for row in rows {
                later |= note_review(provider, &mut out, row, &item.author, created, now)?;
            }
            let more = match pagination::next_link(&headers)? {
                Some(next) => next.is_some(),
                None => rows.len() == size,
            };
            // GitHub's reviews come oldest first: once one counts, or one that would
            // count has no usable time, later pages can't change the first review.
            let settled = !gitea && (out.first.is_some() || out.undated);
            out.limited = more && !settled;
            if !out.limited {
                break;
            }
        }
        // A review newer than the clock allows can't be told apart from a first review
        // only when nothing usable came before it.
        out.undated |= later && out.first.is_none();
        Ok(out)
    }
}

/// Takes one review row into `out`. A state this read does not know is an invalid
/// response, as in the details read; requests and unsubmitted drafts are skipped.
/// Returns whether the review would count but carries a time after `now`.
fn note_review(
    provider: ForgeProvider,
    out: &mut ForgeFirstReview,
    row: &Json,
    author: &str,
    created: Option<u64>,
    now: u64,
) -> Result<bool> {
    let state = row.str("state").ok_or(Error::Forge(ForgeFailure::InvalidResponse))?;
    let counted = if provider == ForgeProvider::Gitea {
        match state {
            "APPROVED" | "REQUEST_CHANGES" | "COMMENT" => true,
            "PENDING" | "REQUEST_REVIEW" => false,
            _ => return fail(ForgeFailure::InvalidResponse),
        }
    } else {
        match state {
            "APPROVED" | "CHANGES_REQUESTED" | "COMMENTED" | "DISMISSED" => true,
            "PENDING" => false,
            _ => return fail(ForgeFailure::InvalidResponse),
        }
    };
    if !counted {
        return Ok(false);
    }
    let user = row.get("user").filter(|u| !matches!(u, Json::Null));
    let login = user.and_then(|u| u.str("login")).unwrap_or("");
    if !login.is_empty() && login.eq_ignore_ascii_case(author) {
        return Ok(false);
    }
    let name = if login.is_empty() { "Deleted user".to_owned() } else { crate::display_text::bounded(login, 256).text };
    if !out.reviewers.contains(&name) {
        out.reviewers.push(name.clone());
    }
    if user.is_some_and(|u| bot(u, login)) {
        return Ok(false);
    }
    // Missing, malformed or before the pull request existed: it could have been the
    // first review, so the first review can't be told.
    let Some(time) = row.str("submitted_at").and_then(date).filter(|t| created.is_none_or(|c| *t >= c)) else {
        out.undated = true;
        return Ok(false);
    };
    // Newer than the clock allows: it can't be earlier than a usable review.
    if time > now {
        return Ok(true);
    }
    if out.first.as_ref().is_none_or(|(_, first)| time < *first) {
        out.first = Some((name, time));
    }
    Ok(false)
}

/// Review figures for a cohort of pull requests.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ForgeReviewMetrics {
    /// Open pull requests (whole sample, like Open now) with a request still waiting,
    /// for the chosen reviewer when there is one.
    pub waiting: usize,
    /// The requests waiting on those pull requests.
    pub requests: usize,
    /// Open pull requests whose row says which requests wait.
    pub requests_known: usize,
    /// Open pull requests whose row does not: GitLab, a malformed row, or a
    /// Bitbucket Cloud page without its reviewers.
    pub requests_unknown: usize,
    /// Pull requests opened in the period, with a usable creation date.
    pub opened: usize,
    /// Of those, the ones whose reviews were read.
    pub read: usize,
    /// Of those read, the ones with a first review: the median's sample.
    pub reviewed: usize,
    pub median_first_review_seconds: Option<u64>,
    /// Read and no review by anyone other than the author yet.
    pub no_review: usize,
    /// Read, but a review's time was unusable, so the first review can't be told.
    pub undated: usize,
    /// Read, but more reviews exist than were read on a host without an order.
    pub limited: usize,
    /// The read failed.
    pub unavailable: usize,
    /// Not read yet.
    pub not_read: usize,
}
impl ForgeReviewMetrics {
    /// `items` is the cohort the other figures use. With a `reviewer`, only the
    /// requests waiting on them count; the first review stays the pull request's.
    pub fn from_items<'a>(
        items: impl IntoIterator<Item = &'a ForgeItem>,
        reads: &HashMap<u64, ForgeReviewRead>,
        reviewer: Option<&ForgeReviewer>,
        since: Option<u64>,
        now: u64,
    ) -> Self {
        let mut out = Self::default();
        let (mut times, mut seen) = (Vec::new(), HashSet::new());
        for item in items {
            if !seen.insert(item.number) {
                continue;
            }
            let Some(t) = &item.timing else { continue };
            if t.open {
                match item.reviewers.as_ref().filter(|r| r.known) {
                    Some(r) => {
                        out.requests_known += 1;
                        let asked = r.waiting.iter().filter(|w| reviewer.is_none_or(|x| *w == x)).count();
                        if asked > 0 {
                            out.waiting += 1;
                            out.requests += asked;
                        }
                    }
                    None => out.requests_unknown += 1,
                }
            }
            let Some(created) = t.created.filter(|c| *c <= now && since.is_none_or(|s| *c >= s)) else {
                continue;
            };
            out.opened += 1;
            match reads.get(&item.number) {
                None => out.not_read += 1,
                Some(ForgeReviewRead::Failed(_)) => out.unavailable += 1,
                Some(ForgeReviewRead::Read(r)) => {
                    out.read += 1;
                    if r.limited {
                        out.limited += 1;
                    } else if r.undated {
                        out.undated += 1;
                    } else if let Some((_, first)) = &r.first {
                        out.reviewed += 1;
                        times.push(first.saturating_sub(created));
                    } else {
                        out.no_review += 1;
                    }
                }
            }
        }
        out.median_first_review_seconds = median(&mut times);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn row(
        n: u64,
        created: Option<u64>,
        merged: Option<u64>,
        open: bool,
        is_merged: bool,
    ) -> ForgeItem {
        ForgeItem {
            number: n,
            reviewers: None,
            timing: Some(ForgePullTiming {
                created,
                merged,
                updated: None,
                open,
                is_merged,
            }),
            title: String::new(),
            state: String::new(),
            author: String::new(),
            source: String::new(),
            target: String::new(),
            url: String::new(),
        }
    }
    #[test]
    fn insights_cohorts_deduplicate_and_keep_missing_dates_out_of_medians() {
        let rows = vec![
            row(1, Some(100), Some(200), false, true),
            row(2, Some(300), Some(500), false, true),
            row(3, Some(50), None, true, false),
            row(4, Some(700), None, false, true),
            row(5, Some(900), Some(850), false, true),
            row(6, Some(2000), None, true, false),
            // A future creation date must not hide the backwards merge date.
            row(7, Some(2000), Some(800), false, true),
            row(1, Some(0), Some(999), false, true),
        ];
        let m = ForgePullMetrics::from_items(&rows, Some(250), 1000);
        assert_eq!((m.loaded, m.opened, m.merged, m.open_now), (7, 3, 1, 2));
        assert_eq!((m.median_merge_seconds, m.merge_samples), (Some(200), 1));
        assert_eq!(
            (m.median_open_seconds, m.open_samples, m.incomplete_dates),
            (Some(950), 1, 4)
        );
        // The rows a list marks are the ones the count names.
        let marked = rows[..7]
            .iter()
            .filter(|r| r.timing.as_ref().unwrap().incomplete(1000))
            .map(|r| r.number)
            .collect::<Vec<_>>();
        assert_eq!(marked, [4, 5, 6, 7]);
        assert_eq!(median(&mut [u64::MAX - 2, u64::MAX]), Some(u64::MAX - 1));
        assert_eq!(
            ForgePullMetrics::from_items(&[], None, 1000).median_merge_seconds,
            None
        );
    }
    fn json(s: &str) -> Json {
        crate::hosting::json::parse(s.as_bytes()).unwrap()
    }
    fn who(list: &[ForgeReviewer]) -> Vec<String> {
        list.iter().map(|r| format!("{}{}", if r.team { "team:" } else { "" }, r.name)).collect()
    }
    #[test]
    fn insights_reviewers_come_from_each_hosts_list_row() {
        let github = review_requests(
            ForgeProvider::GitHub,
            "team",
            &json(r#"{"requested_reviewers":[{"login":"Bob"},{"login":"Bob"}],"requested_teams":[{"slug":"core","name":"Core"}]}"#),
        )
        .unwrap();
        assert!(github.known);
        assert_eq!(who(&github.waiting), ["Bob", "team:team/core"]);
        // A search result has no reviewer fields: nothing is claimed either way.
        assert_eq!(review_requests(ForgeProvider::GitHub, "team", &json(r#"{"number":1}"#)), None);
        // Gitea's list names whoever's latest review is a request, an approval or a
        // rejection: reviewers, but not who still waits.
        let gitea = review_requests(
            ForgeProvider::Gitea,
            "org",
            &json(r#"{"requested_reviewers":[{"login":"Bob"}],"requested_reviewers_teams":null}"#),
        )
        .unwrap();
        assert_eq!((gitea.known, who(&gitea.assigned), gitea.waiting.len()), (false, vec!["Bob".to_owned()], 0));
        let empty = review_requests(ForgeProvider::Gitea, "org", &json(r#"{"requested_reviewers":null}"#)).unwrap();
        assert_eq!((empty.known, empty.assigned.len()), (false, 0));
        let gitlab = review_requests(
            ForgeProvider::GitLab,
            "group",
            &json(r#"{"reviewers":[{"username":"ann","state":"active"}]}"#),
        )
        .unwrap();
        assert!(!gitlab.known, "GitLab's state is the account's, not a review");
        assert_eq!((who(&gitlab.assigned), gitlab.waiting.len()), (vec!["ann".to_owned()], 0));
        assert_eq!(review_requests(ForgeProvider::Bitbucket, "w", &json(r#"{"id":1}"#)), None);
        let cloud = review_requests(
            ForgeProvider::Bitbucket,
            "w",
            &json(r#"{"reviewers":[{"display_name":"Bob","uuid":"{1}"},{"display_name":"Cy","uuid":"{2}"}],
                "participants":[{"user":{"display_name":"Bob","uuid":"{1}"},"role":"REVIEWER","approved":true,"state":"approved"},
                {"user":{"display_name":"Cy","uuid":"{2}"},"role":"REVIEWER","approved":false,"state":null},
                {"user":{"display_name":"Dee","uuid":"{3}"},"role":"PARTICIPANT","approved":false,"state":"changes_requested"}]}"#),
        )
        .unwrap();
        assert_eq!((cloud.known, who(&cloud.waiting), who(&cloud.reviewed)),
            (true, vec!["Cy".to_owned()], vec!["Bob".to_owned(), "Dee".to_owned()]));
        let server = review_requests(
            ForgeProvider::BitbucketServer,
            "PRJ",
            &json(r#"{"reviewers":[{"user":{"displayName":"Bob"},"status":"UNAPPROVED"},{"user":{"name":"cy"},"status":"NEEDS_WORK"},{"user":{"displayName":"Dee"},"status":"APPROVED"}]}"#),
        )
        .unwrap();
        assert_eq!((server.known, who(&server.waiting), who(&server.reviewed)),
            (true, vec!["Bob".to_owned()], vec!["cy".to_owned(), "Dee".to_owned()]));
        let azure = review_requests(
            ForgeProvider::Azure,
            "project",
            &json(r#"{"reviewers":[{"displayName":"Bob","vote":0},{"displayName":"Project Team","vote":0,"isContainer":true},
                {"displayName":"Cy","vote":10},{"displayName":"Dee","vote":0,"hasDeclined":true},{"displayName":"Eve","vote":-5}]}"#),
        )
        .unwrap();
        assert_eq!((azure.known, who(&azure.waiting), who(&azure.reviewed)),
            (true, vec!["Bob".to_owned(), "team:Project Team".to_owned()], vec!["Cy".to_owned(), "Eve".to_owned()]));
        // Malformed, unknown or too many entries leave the row's state unknown
        // instead of undercounting what waits.
        for row in [
            r#"{"requested_reviewers":"Bob"}"#,
            r#"{"requested_reviewers":[{"name":"no login"}]}"#,
            r#"{"requested_reviewers":[],"requested_teams":[{"name":"no slug"}]}"#,
        ] {
            assert!(!review_requests(ForgeProvider::GitHub, "t", &json(row)).unwrap().known, "{row}");
        }
        assert!(!review_requests(ForgeProvider::BitbucketServer, "p", &json(r#"{"reviewers":[{"user":{"name":"x"},"status":"LATER"}]}"#)).unwrap().known);
        assert!(!review_requests(ForgeProvider::Azure, "p", &json(r#"{"reviewers":[{"displayName":"x","vote":3}]}"#)).unwrap().known);
        let many = format!(r#"{{"requested_reviewers":[{}]}}"#,
            (0..=MAX_REVIEWERS).map(|n| format!(r#"{{"login":"u{n}"}}"#)).collect::<Vec<_>>().join(","));
        let many = review_requests(ForgeProvider::GitHub, "t", &json(&many)).unwrap();
        assert_eq!((many.known, many.waiting.len()), (false, MAX_REVIEWERS));
    }
    #[test]
    fn insights_first_review_skips_the_author_bots_drafts_and_unusable_times() {
        let created = 1_709_251_200; // 2024-03-01T00:00:00Z
        let now = created + 86_400;
        let mut out = ForgeFirstReview::default();
        for row in [
            r#"{"state":"COMMENTED","user":{"login":"Fixture"},"submitted_at":"2024-03-01T00:05:00Z"}"#,
            r#"{"state":"PENDING","user":{"login":"Ann"}}"#,
            r#"{"state":"APPROVED","user":{"login":"helper","type":"Bot"},"submitted_at":"2024-03-01T00:06:00Z"}"#,
            r#"{"state":"COMMENTED","user":{"login":"copilot[bot]"},"submitted_at":"2024-03-01T00:07:00Z"}"#,
            r#"{"state":"CHANGES_REQUESTED","user":{"login":"Bob"},"submitted_at":"2024-03-01T02:00:00Z"}"#,
            r#"{"state":"APPROVED","user":{"login":"Ann"},"submitted_at":"2024-03-01T01:00:00+02:00"}"#,
            r#"{"state":"DISMISSED","user":null,"submitted_at":"2024-03-01T05:00:00Z"}"#,
        ] {
            note_review(ForgeProvider::GitHub, &mut out, &json(row), "fixture", Some(created), now).unwrap();
        }
        // Ann's time is 23:00 the day before, before the pull request existed: unusable.
        assert_eq!(out.first, Some(("Bob".to_owned(), created + 7200)));
        assert!(out.undated);
        assert_eq!(out.reviewers, ["helper", "copilot[bot]", "Bob", "Ann", "Deleted user"]);
        let mut gitea = ForgeFirstReview::default();
        for row in [
            r#"{"state":"REQUEST_REVIEW","user":{"login":"Bob"}}"#,
            r#"{"state":"COMMENT","user":{"login":"Bob"},"submitted_at":"2024-03-01T03:00:00Z"}"#,
            r#"{"state":"REQUEST_CHANGES","user":{"login":"Cy"},"submitted_at":"2024-03-01T01:00:00Z"}"#,
        ] {
            note_review(ForgeProvider::Gitea, &mut gitea, &json(row), "fixture", Some(created), now).unwrap();
        }
        assert_eq!((gitea.first, gitea.undated), (Some(("Cy".to_owned(), created + 3600)), false));
        for (provider, row) in [
            (ForgeProvider::GitHub, r#"{"state":"REQUEST_REVIEW","user":{"login":"Bob"}}"#),
            (ForgeProvider::Gitea, r#"{"state":"CHANGES_REQUESTED","user":{"login":"Bob"}}"#),
            (ForgeProvider::GitHub, r#"{"user":{"login":"Bob"}}"#),
        ] {
            let mut out = ForgeFirstReview::default();
            assert!(matches!(
                note_review(provider, &mut out, &json(row), "fixture", Some(created), now),
                Err(Error::Forge(ForgeFailure::InvalidResponse))
            ));
        }
        // A time after `now` is reported, not taken: it can't be earlier than a usable
        // review, and the read marks the pull request undated only when none is usable.
        let mut future = ForgeFirstReview::default();
        let later = note_review(ForgeProvider::GitHub, &mut future, &json(r#"{"state":"APPROVED","user":{"login":"Bob"},"submitted_at":"2024-03-09T00:00:00Z"}"#), "fixture", Some(created), now).unwrap();
        assert!(later);
        assert_eq!((future.first, future.undated), (None, false));
    }
    #[test]
    fn insights_review_metrics_count_requests_and_label_every_missing_read() {
        let person = |name: &str| ForgeReviewer { team: false, name: name.into() };
        let team = ForgeReviewer { team: true, name: "team/core".into() };
        let known = |waiting: Vec<ForgeReviewer>| ForgeReviewRequests { known: true, waiting, ..Default::default() };
        let mut rows = vec![
            row(1, Some(100), None, true, false),
            row(2, Some(100), None, true, false),
            row(3, Some(100), None, true, false),
            row(4, Some(300), Some(900), false, true),
            row(5, Some(300), None, true, false),
            row(6, Some(400), None, false, true),
            row(7, Some(500), None, true, false),
            row(8, Some(600), None, true, false),
            row(9, Some(700), None, true, false),
            row(10, Some(800), Some(900), false, true),
            row(4, Some(0), None, true, false),
        ];
        rows[0].reviewers = Some(known(vec![person("Bob"), team.clone()]));
        rows[1].reviewers = Some(known(vec![]));
        rows[2].reviewers = Some(ForgeReviewRequests { assigned: vec![person("Bob")], ..Default::default() });
        rows[4].reviewers = Some(known(vec![person("Cy")]));
        let read = |first: Option<u64>, undated: bool, limited: bool| ForgeReviewRead::Read(ForgeFirstReview {
            first: first.map(|t| ("Ann".to_owned(), t)),
            reviewers: vec!["Ann".into()],
            undated,
            limited,
        });
        let reads = HashMap::from([
            (4, read(Some(1300), false, false)),
            (5, read(None, false, false)),
            (6, read(Some(700), true, false)),
            (7, ForgeReviewRead::Failed(ForgeFailure::NotFound)),
            (9, read(Some(800), false, true)),
            (10, read(Some(1000), false, false)),
        ]);
        let m = ForgeReviewMetrics::from_items(&rows, &reads, None, Some(250), 2000);
        assert_eq!((m.waiting, m.requests, m.requests_known, m.requests_unknown), (2, 3, 3, 4));
        assert_eq!((m.opened, m.read, m.reviewed, m.median_first_review_seconds), (7, 5, 2, Some(600)));
        assert_eq!((m.no_review, m.undated, m.limited, m.unavailable, m.not_read), (1, 1, 1, 1, 1));
        // A chosen reviewer counts only the requests waiting on them.
        let bob = ForgeReviewMetrics::from_items(&rows, &reads, Some(&person("Bob")), Some(250), 2000);
        assert_eq!((bob.waiting, bob.requests), (1, 1));
        let core = ForgeReviewMetrics::from_items(&rows, &reads, Some(&team), None, 2000);
        assert_eq!((core.waiting, core.requests, core.opened), (1, 1, 10));
        assert_eq!(ForgeReviewMetrics::from_items(&[], &reads, None, None, 0).median_first_review_seconds, None);
    }
    #[test]
    fn insights_dates_are_strict_and_timezone_aware() {
        let z = date("2024-02-29T10:30:00Z").unwrap();
        assert_eq!(date("2024-02-29T18:30:00.123+08:00"), Some(z));
        for s in [
            "2023-02-29T10:30:00Z",
            "2024-02-30T10:30:00Z",
            "2024-02-29T10:30:00.Z",
            "2024-02-29T10:30:00+25:00",
            "資料資料資料資料資料資料資料",
            "2024-02-29T10:30:00",
        ] {
            assert_eq!(date(s), None, "{s}");
        }
    }
    #[test]
    fn insights_host_dates_never_use_updated_time_as_merge_time() {
        let parse = |s: &str| crate::hosting::json::parse(s.as_bytes()).unwrap();
        let github = parse(
            r#"{"state":"closed","created_at":"2024-02-28T10:00:00Z","merged_at":"2024-02-29T10:00:00Z"}"#,
        );
        for p in [
            ForgeProvider::GitHub,
            ForgeProvider::GitHubEnterprise,
            ForgeProvider::Gitea,
            ForgeProvider::GitLab,
        ] {
            let mut t = timing(p, &github);
            if p == ForgeProvider::GitLab {
                t = timing(
                    p,
                    &parse(
                        r#"{"state":"merged","created_at":"2024-02-28T10:00:00Z","merged_at":"2024-02-29T10:00:00Z"}"#,
                    ),
                );
            }
            assert!(t.is_merged);
            assert_eq!(t.merged.unwrap() - t.created.unwrap(), 86400);
        }
        let bb = timing(
            ForgeProvider::Bitbucket,
            &parse(
                r#"{"state":"MERGED","created_on":"2024-02-28T10:00:00Z","updated_on":"2024-03-01T10:00:00Z"}"#,
            ),
        );
        assert!(bb.is_merged);
        assert!(bb.created.is_some());
        assert_eq!(bb.merged, None);
        let server = timing(
            ForgeProvider::BitbucketServer,
            &parse(
                r#"{"state":"MERGED","createdDate":100000,"closedDate":250000,"updatedDate":999999}"#,
            ),
        );
        assert_eq!((server.created, server.merged), (Some(100), Some(250)));
        // Update times are read for coverage only; each host names them its own way.
        assert_eq!(server.updated, Some(999));
        assert_eq!(bb.updated, date("2024-03-01T10:00:00Z"));
        let updated = parse(r#"{"state":"open","created_at":"2024-02-28T10:00:00Z","updated_at":"2024-03-02T10:00:00Z"}"#);
        for p in [ForgeProvider::GitHub, ForgeProvider::GitHubEnterprise, ForgeProvider::Gitea, ForgeProvider::GitLab] {
            assert_eq!(timing(p, &updated).updated, date("2024-03-02T10:00:00Z"), "{p:?}");
        }
        for p in [ForgeProvider::Azure, ForgeProvider::AzureServer] {
            let t = timing(
                p,
                &parse(
                    r#"{"status":"completed","creationDate":"2024-02-28T10:00:00Z","closedDate":"2024-02-29T10:00:00Z"}"#,
                ),
            );
            assert!(t.is_merged);
            assert_eq!(t.merged.unwrap() - t.created.unwrap(), 86400);
            assert_eq!(t.updated, None, "Azure lists have no update time");
        }
    }
    fn dated(n: u64, created: u64, merged: Option<u64>, updated: Option<u64>) -> ForgeItem {
        let mut item = row(n, Some(created), merged, merged.is_none(), merged.is_some());
        item.timing.as_mut().unwrap().updated = updated;
        item
    }
    #[test]
    fn insights_coverage_reaches_back_to_the_oldest_update_on_hosts_that_sort_by_it() {
        let rows = [dated(1, 50, None, Some(900)), dated(2, 10, Some(40), Some(400))];
        for p in [ForgeProvider::GitHub, ForgeProvider::GitHubEnterprise, ForgeProvider::GitLab, ForgeProvider::Gitea, ForgeProvider::Bitbucket] {
            assert_eq!(ForgePullCoverage::of(&rows, p, true), ForgePullCoverage::After(400), "{p:?}");
            assert_eq!(ForgePullCoverage::of(&rows, p, false), ForgePullCoverage::Complete);
        }
        // Hosts whose order isn't known, or a row without an update time, say nothing.
        for p in [ForgeProvider::Azure, ForgeProvider::AzureServer, ForgeProvider::BitbucketServer] {
            assert_eq!(ForgePullCoverage::of(&rows, p, true), ForgePullCoverage::Unknown, "{p:?}");
            assert_eq!(ForgePullCoverage::of(&rows, p, false), ForgePullCoverage::Complete);
        }
        let mut missing = rows.to_vec();
        missing.push(dated(3, 60, None, None));
        assert_eq!(ForgePullCoverage::of(&missing, ForgeProvider::GitHub, true), ForgePullCoverage::Unknown);
        assert_eq!(ForgePullCoverage::of(&[], ForgeProvider::GitHub, true), ForgePullCoverage::Unknown);
        // Rows updated exactly at the oldest update may continue on the next page.
        let after = ForgePullCoverage::After(400);
        assert!((after.covers(401), after.covers(400), after.covers(10)) == (true, false, false));
        assert!(ForgePullCoverage::Complete.covers(0) && !ForgePullCoverage::Unknown.covers(u64::MAX));
    }
    #[test]
    fn insights_periods_count_their_own_openings_and_usable_merges_once() {
        let rows = vec![
            dated(1, 100, Some(300), None),
            dated(2, 250, Some(450), None),
            dated(3, 299, None, None),
            dated(4, 120, None, None),
            // Merged before it was opened, and merged in the future: no merge counts.
            dated(5, 280, Some(260), None),
            dated(6, 280, Some(2000), None),
            dated(1, 0, Some(299), None),
        ];
        let previous = ForgePullCounts::between(&rows, 100, 300, 1000);
        // Merged exactly at the end of a period belongs to the next one.
        assert_eq!((previous.opened, previous.merged), (6, 0));
        let current = ForgePullCounts::between(&rows, 300, 1001, 1000);
        assert_eq!((current.opened, current.merged, current.merge_samples), (0, 2, 2));
        assert_eq!(current.median_merge_seconds, Some(200));
        // The current period agrees with the tiles' metrics.
        let m = ForgePullMetrics::from_items(&rows, Some(300), 1000);
        assert_eq!((m.opened, m.merged, m.median_merge_seconds), (0, 2, Some(200)));
        assert_eq!(ForgePullCounts::between(&[], 0, 10, 10), ForgePullCounts::default());
    }
    #[test]
    fn insights_trends_bucket_by_week_month_and_year_and_add_up_to_the_period() {
        const DAY: u64 = 86_400;
        // 2026-11-12 is a Thursday; the 90 days start on Friday 2026-08-14.
        let now = crate::statistics::days_from_civil(2026, 11, 12) as u64 * DAY + 13 * 3600;
        let since = now - 90 * DAY;
        let rows = vec![
            dated(1, since + 3600, Some(since + DAY), None),
            dated(2, now - 2 * DAY, None, None),
            dated(3, since - DAY, Some(now), None),
            dated(4, since - 10 * DAY, Some(since - DAY), None),
            dated(3, since + 2 * DAY, None, None),
        ];
        let t = ForgePullTrend::from_items(&rows, Some(since), now);
        assert_eq!(t.unit, ForgeTrendUnit::Week);
        assert_eq!(t.buckets.len(), 14);
        // The first week starts with the period, the others on Mondays.
        assert_eq!(t.buckets[0].start, since);
        let monday = crate::statistics::days_from_civil(2026, 8, 17) as u64 * DAY;
        assert_eq!(t.buckets[1].start, monday);
        assert!(t.buckets.windows(2).skip(1).all(|w| w[1].start - w[0].start == 7 * DAY));
        assert_eq!((t.buckets[0].opened, t.buckets[0].merged), (1, 1));
        assert_eq!((t.buckets[13].opened, t.buckets[13].merged), (1, 1));
        let m = ForgePullMetrics::from_items(&rows, Some(since), now);
        let sum = |f: fn(&ForgeTrendBucket) -> usize| t.buckets.iter().map(f).sum::<usize>();
        assert_eq!((sum(|b| b.opened), sum(|b| b.merged)), (m.opened, m.merged));
        // All history: from the first event; months past 53 weeks, years past 48 months.
        let old = |years: u64| vec![dated(1, now - years * 365 * DAY, None, None), dated(2, now, None, None)];
        let months = ForgePullTrend::from_items(&old(2), None, now);
        assert_eq!((months.unit, months.buckets.len()), (ForgeTrendUnit::Month, 25));
        assert_eq!((months.buckets[0].opened, months.buckets[24].opened), (1, 1));
        let first = crate::statistics::days_from_civil(2024, 12, 1) as u64 * DAY;
        assert_eq!(months.buckets[1].start, first);
        let years = ForgePullTrend::from_items(&old(5), None, now);
        assert_eq!((years.unit, years.buckets.len()), (ForgeTrendUnit::Year, 6));
        assert_eq!(years.buckets[1].start, crate::statistics::days_from_civil(2022, 1, 1) as u64 * DAY);
        let weeks = ForgePullTrend::from_items(&old(0), None, now);
        assert_eq!((weeks.unit, weeks.buckets.len()), (ForgeTrendUnit::Week, 1));
        assert_eq!(ForgePullTrend::from_items(&[], None, now), ForgePullTrend::default());
    }
}
