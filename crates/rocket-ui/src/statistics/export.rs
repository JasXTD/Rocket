//! An explicitly reviewed, immutable Insights sample. No credentials or PR bodies.
//! Save publishes a complete new file without overwriting a racing destination.
use super::*;
use crate::{
    backend::Backend,
    i18n::{mark, tr, trf, trn},
    pages::{Part, ReviewHeader},
};
use serde_json::{Value, json};
use std::{
    fs,
    io::Write,
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Format {
    #[default]
    Csv,
    Json,
}
impl Format {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Csv => "csv",
            Self::Json => "json",
        }
    }
}

#[derive(Clone)]
pub struct Snapshot {
    pub items: Vec<ForgeItem>,
    pub repository: String,
    pub provider: String,
    pub server: String,
    pub read_at: u64,
    pub pages: usize,
    pub loaded: usize,
    pub partial: bool,
    pub author: Option<String>,
    pub reviewer: Option<ForgeReviewer>,
    pub state: &'static str,
    pub period: &'static str,
    pub metrics: ForgePullMetrics,
    pub review_metrics: ForgeReviewMetrics,
    /// Whether the host's reviews carry their times, so Read reviews could time them.
    pub times_reviews: bool,
    /// Each row's reviews as read, beside `items`.
    pub reviews: Vec<Option<ForgeReviewRead>>,
}
impl Snapshot {
    fn new(s: &pulls::State) -> Self {
        let (provider, server) = s.source().unwrap_or(("", ""));
        let items: Vec<ForgeItem> = s.visible().cloned().collect();
        Self {
            reviews: items.iter().map(|i| s.reviews.get(&i.number).cloned()).collect(),
            items,
            repository: s.repository.clone().unwrap_or_default(),
            provider: provider.into(),
            server: server.into(),
            read_at: s.read_at,
            pages: s.pages,
            loaded: s.items.len(),
            partial: s.more || s.limited,
            author: s.author.clone(),
            reviewer: s.reviewer.clone(),
            state: match s.show {
                1 => "open",
                2 => "merged",
                3 => "waiting",
                _ => "all",
            },
            period: s.period.unwrap_or(Period::Days30).key(),
            metrics: s.metrics.clone(),
            review_metrics: s.review_metrics.clone(),
            times_reviews: s.times_reviews(),
        }
    }
    /// What is known of a row's reviews, in one word a blank time can't be mistaken for.
    fn reviews_word(&self, read: Option<&ForgeReviewRead>) -> &'static str {
        match read {
            _ if !self.times_reviews => "not_measured",
            None => "not_read",
            Some(ForgeReviewRead::Failed(_)) => "unavailable",
            Some(ForgeReviewRead::Read(r)) if r.limited => "incomplete",
            Some(ForgeReviewRead::Read(r)) if r.undated => "no_usable_time",
            Some(ForgeReviewRead::Read(_)) => "read",
        }
    }
    /// The first review as counted: none unless the read could tell it.
    fn first_review(read: Option<&ForgeReviewRead>) -> Option<&(String, u64)> {
        match read {
            Some(ForgeReviewRead::Read(r)) if !r.limited && !r.undated => r.first.as_ref(),
            _ => None,
        }
    }
    /// Who an open row still waits on, as the Waiting filter counts it; nobody for a
    /// closed or merged one. Where the list does not say, the word for why: the row
    /// has no reviewer fields, or names reviewers without saying who still waits.
    fn requests(item: &ForgeItem) -> std::result::Result<Vec<&ForgeReviewer>, &'static str> {
        let reviewers = item.reviewers.as_ref().ok_or("not_in_list")?;
        if !item.timing.as_ref().is_some_and(|t| t.open) {
            return Ok(Vec::new());
        }
        if !reviewers.known {
            return Err("no_review_state");
        }
        Ok(reviewers.waiting.iter().collect())
    }
    fn json(&self) -> Value {
        let m = &self.metrics;
        let r = &self.review_metrics;
        let who = |w: &ForgeReviewer| json!({"name":w.name,"team":w.team});
        json!({"schema_version":1,"provider":self.provider,"server":self.server,"repository":self.repository,"snapshot_unix_seconds":self.read_at,
            "coverage":{"loaded_unique_prs":self.loaded,"pages":self.pages,"partial":self.partial,"exported_rows":self.items.len(),"atomic_snapshot":false},
            "filters":{"author":self.author,"reviewer":self.reviewer.as_ref().map(who),"state":self.state,"summary_period":self.period,"rows_period":"all_loaded_dates"},
            // The scope keeps the word earlier exports used unless a reviewer narrows it.
            "metrics":{"scope":if self.reviewer.is_some() { "author_and_reviewer_filtered_loaded_sample_before_state_filter" } else { "author_filtered_loaded_sample_before_state_filter" },"loaded":m.loaded,"opened":m.opened,"merged":m.merged,"open_now":m.open_now,
                "merge_samples":m.merge_samples,"median_merge_seconds":m.median_merge_seconds,"open_samples":m.open_samples,"median_open_seconds":m.median_open_seconds,"incomplete_dates":m.incomplete_dates,
                "waiting_for_review":r.waiting,"review_requests":r.requests,"open_without_review_state":r.requests_unknown,
                "first_review":{"measured":self.times_reviews,"opened":r.opened,"read":r.read,"samples":r.reviewed,"median_seconds":r.median_first_review_seconds,
                    "no_review_yet":r.no_review,"no_usable_time":r.undated,"incomplete":r.limited,"unavailable":r.unavailable,"not_read":r.not_read}},
            "items":self.items.iter().zip(&self.reviews).map(|(i, read)| json!({"number":i.number,"title":i.title,"author":i.author,"state":i.state,"source":i.source,"target":i.target,
                "created_unix_seconds":i.timing.as_ref().and_then(|t|t.created),"merged_unix_seconds":i.timing.as_ref().and_then(|t|t.merged),
                "open":i.timing.as_ref().map(|t|t.open),"merged":i.timing.as_ref().map(|t|t.is_merged),
                "review_requests":Self::requests(i).ok().map(|list| list.into_iter().map(who).collect::<Vec<_>>()),
                "review_requests_unknown":Self::requests(i).err(),
                "reviews":self.reviews_word(read.as_ref()),
                "first_review_unix_seconds":Self::first_review(read.as_ref()).map(|f| f.1),
                "first_reviewer":Self::first_review(read.as_ref()).map(|f| &f.0)})).collect::<Vec<_>>()})
    }
    fn encode(&self, format: Format, cancel: &CancellationToken) -> Result<Vec<u8>> {
        cancel.check()?;
        let out = match format {
            Format::Json => serde_json::to_vec_pretty(&self.json())
                .map_err(|_| Error::InvalidInput("Could not encode Insights JSON.".into()))?,
            Format::Csv => {
                // UTF-8 BOM helps spreadsheet applications recognize Unicode names.
                let mut out = String::from(
                    // Review columns come last, so earlier columns keep their places.
                    "\u{feff}schema_version,provider,server,repository,snapshot_unix_seconds,loaded_unique_prs,pages,partial,author_filter,state_filter,summary_period,rows_period,number,title,author,state,source,target,created_unix_seconds,merged_unix_seconds,open,merged,reviewer_filter,review_requests,reviews,first_review_unix_seconds,first_reviewer\r\n",
                );
                for (i, read) in self.items.iter().zip(&self.reviews) {
                    cancel.check()?;
                    let first = Self::first_review(read.as_ref());
                    // Names joined with "; "; a row whose list does not say says why.
                    let requests = match Self::requests(i) {
                        Ok(list) => list.iter().map(|w| w.name.as_str()).collect::<Vec<_>>().join("; "),
                        Err(word) => word.to_owned(),
                    };
                    let optional = |v: Option<u64>| v.map(|n| n.to_string()).unwrap_or_default();
                    let boolean = |v: Option<bool>| v.map(|b| b.to_string()).unwrap_or_default();
                    let fields = [
                        "1".into(),
                        self.provider.clone(),
                        self.server.clone(),
                        self.repository.clone(),
                        self.read_at.to_string(),
                        self.loaded.to_string(),
                        self.pages.to_string(),
                        self.partial.to_string(),
                        self.author.clone().unwrap_or_default(),
                        self.state.into(),
                        self.period.into(),
                        "all_loaded_dates".into(),
                        i.number.to_string(),
                        i.title.clone(),
                        i.author.clone(),
                        i.state.clone(),
                        i.source.clone(),
                        i.target.clone(),
                        optional(i.timing.as_ref().and_then(|t| t.created)),
                        optional(i.timing.as_ref().and_then(|t| t.merged)),
                        boolean(i.timing.as_ref().map(|t| t.open)),
                        boolean(i.timing.as_ref().map(|t| t.is_merged)),
                        self.reviewer.as_ref().map(|w| w.name.clone()).unwrap_or_default(),
                        requests,
                        self.reviews_word(read.as_ref()).into(),
                        optional(first.map(|f| f.1)),
                        first.map(|f| f.0.clone()).unwrap_or_default(),
                    ];
                    out.push_str(
                        &fields
                            .iter()
                            .map(|s| csv_cell(s))
                            .collect::<Vec<_>>()
                            .join(","),
                    );
                    out.push_str("\r\n");
                }
                out.into_bytes()
            }
        };
        cancel.check()?;
        if out.len() > 16 * 1024 * 1024 {
            return Err(Error::InvalidInput(
                "Insights export exceeds 16 MiB. Narrow the author, reviewer or state filters.".into(),
            ));
        }
        Ok(out)
    }
}

/// What an activity export holds: the commit activity shown, as it was read. Names and
/// addresses are the repository's own (after .mailmap); its path is never included.
#[derive(Clone)]
pub struct ActivitySnapshot {
    pub activity: Arc<CommitActivity>,
    /// The repository's folder name.
    pub repository: String,
    /// The period and the history the activity was read for.
    pub scope: (Period, ActivityScope),
    pub read_at: u64,
}

/// The table an activity CSV holds; JSON holds every table.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Table {
    #[default]
    Contributors,
    Weeks,
}

#[derive(Clone)]
pub enum Content {
    // Boxed: a pull request snapshot is much larger than an activity one.
    Pulls(Box<Snapshot>),
    Activity(ActivitySnapshot),
}
impl Content {
    fn encode(&self, format: Format, table: Table, emails: bool, cancel: &CancellationToken) -> Result<Vec<u8>> {
        match self {
            Self::Pulls(s) => s.encode(format, cancel),
            Self::Activity(a) => a.encode(format, table, emails, cancel),
        }
    }
}

impl ActivitySnapshot {
    fn branches(&self) -> &'static str {
        if self.scope.1.all_branches { "all_branches" } else { "this_branch" }
    }
    /// How many weeks [`weeks`](Self::weeks) lists, worked out without listing them.
    fn week_count(&self) -> usize {
        match (self.activity.weeks.first(), self.activity.weeks.last()) {
            (Some(&(first, _)), Some(&(last, _))) => usize::try_from((last - first) / 7).map_or(usize::MAX, |n| n.saturating_add(1)),
            _ => 0,
        }
    }
    /// Every week from the first to the last with commits, empty ones included: the
    /// day its Monday falls on ("2026-08-24") and its commits. Listed only while saving,
    /// where it can be stopped; a span no export could hold is refused before it is listed.
    fn weeks(&self, cancel: &CancellationToken) -> Result<Vec<(String, usize)>> {
        // Far more weeks than 16 MiB of rows can hold (about 3,800 years).
        const MOST: usize = 200_000;
        if self.week_count() > MOST {
            return Err(Error::InvalidInput("Insights export exceeds 16 MiB.".into()));
        }
        let a = &self.activity;
        let counts: HashMap<i64, usize> = a.weeks.iter().copied().collect();
        let (Some(&(first, _)), Some(&(last, _))) = (a.weeks.first(), a.weeks.last()) else {
            return Ok(Vec::new());
        };
        let mut out = Vec::with_capacity(self.week_count());
        for (i, monday) in (first..=last).step_by(7).enumerate() {
            if i % 4096 == 0 {
                cancel.check()?;
            }
            out.push((date(monday * 86_400, 0)[..10].to_owned(), counts.get(&monday).copied().unwrap_or(0)));
        }
        Ok(out)
    }
    fn json(&self, emails: bool, cancel: &CancellationToken) -> Result<Value> {
        let a = &self.activity;
        let weeks = self.weeks(cancel)?;
        let text = |b: &[u8]| String::from_utf8_lossy(b).into_owned();
        Ok(json!({"schema_version":1,"kind":"commit_activity","repository":self.repository,"snapshot_unix_seconds":self.read_at,
            "scope":{"branches":self.branches(),"period":self.scope.0.key(),"since_unix_seconds":self.scope.1.since,
                "dates":"author_dates","identities":"mailmap_grouped_by_email","email_addresses":emails},
            "coverage":{"commits_counted":a.commits,"commit_limit":ACTIVITY_COMMITS,"limited":a.limited},
            "totals":{"commits":a.commits,"merges":a.merges,"contributors":a.authors.len(),"active_days":a.days,
                "first_unix_seconds":a.first,"last_unix_seconds":a.last},
            "contributors":a.authors.iter().map(|x| if emails {
                json!({"name":text(&x.name),"email":text(&x.email),"commits":x.commits})
            } else {
                json!({"name":text(&x.name),"commits":x.commits})
            }).collect::<Vec<_>>(),
            "weeks":weeks.into_iter().map(|(start, commits)| json!({"start":start,"commits":commits})).collect::<Vec<_>>(),
            "months":a.months.iter().map(|(year, month, commits)| json!({"year":year,"month":month,"commits":commits})).collect::<Vec<_>>(),
            "weekday_order":"monday_first","weekdays":a.weekdays,"hours":a.hours,"weekday_hours":a.heat}))
    }
    fn encode(&self, format: Format, table: Table, emails: bool, cancel: &CancellationToken) -> Result<Vec<u8>> {
        cancel.check()?;
        let out = match format {
            Format::Json => serde_json::to_vec_pretty(&self.json(emails, cancel)?)
                .map_err(|_| Error::InvalidInput("Could not encode Insights JSON.".into()))?,
            Format::Csv => {
                let scope = [
                    "1".to_owned(),
                    self.repository.clone(),
                    self.branches().into(),
                    self.scope.0.key().into(),
                    self.scope.1.since.map(|n| n.to_string()).unwrap_or_default(),
                    self.read_at.to_string(),
                    self.activity.commits.to_string(),
                    self.activity.limited.to_string(),
                ];
                let mut head = String::from("\u{feff}schema_version,repository,branches,period,since_unix_seconds,snapshot_unix_seconds,commits_counted,limited,");
                let rows: Vec<Vec<String>> = match table {
                    Table::Contributors => {
                        head.push_str(if emails { "name,email,commits" } else { "name,commits" });
                        self.activity
                            .authors
                            .iter()
                            .map(|x| {
                                let mut row = vec![String::from_utf8_lossy(&x.name).into_owned()];
                                if emails {
                                    row.push(String::from_utf8_lossy(&x.email).into_owned());
                                }
                                row.push(x.commits.to_string());
                                row
                            })
                            .collect()
                    }
                    Table::Weeks => {
                        head.push_str("week_start,commits");
                        self.weeks(cancel)?.into_iter().map(|(start, n)| vec![start, n.to_string()]).collect()
                    }
                };
                let mut out = head + "\r\n";
                for row in rows {
                    cancel.check()?;
                    let line = scope.iter().chain(&row).map(|s| csv_cell(s)).collect::<Vec<_>>().join(",");
                    out.push_str(&line);
                    out.push_str("\r\n");
                }
                out.into_bytes()
            }
        };
        cancel.check()?;
        if out.len() > 16 * 1024 * 1024 {
            return Err(Error::InvalidInput("Insights export exceeds 16 MiB.".into()));
        }
        Ok(out)
    }
}

fn csv_cell(value: &str) -> String {
    // Quoting alone does not prevent spreadsheet formulas. Preserve the exact value
    // in JSON; prefix dangerous spreadsheet text (including hidden prefixes) with '.
    let significant = value.trim_start_matches(|c: char| {
        c.is_whitespace() || c.is_control() || matches!(c, '\u{feff}' | '\u{200b}')
    });
    let formula =
        significant.starts_with(['=', '+', '-', '@']) || value.starts_with(['\t', '\r', '\n']);
    format!(
        "\"{}{}\"",
        if formula { "'" } else { "" },
        value.replace('"', "\"\"")
    )
}

pub struct State {
    /// What the file holds, frozen when the review opened.
    pub content: Content,
    pub format: Format,
    /// An activity CSV's table.
    pub table: Table,
    /// Whether an activity export includes contributors' email addresses.
    pub emails: bool,
    pub dir: String,
    pub name: String,
    pub stopping: bool,
    pub error: Option<String>,
    pub notice: Option<String>,
    /// The name of the file the last save created, for the footer.
    pub saved: Option<String>,
    /// The name the running save writes.
    saving: Option<String>,
    task: Option<WriteTask<()>>,
}
impl State {
    pub(super) fn new(s: &pulls::State, dir: String) -> Self {
        Self::with(Content::Pulls(Box::new(Snapshot::new(s))), dir)
    }
    pub(super) fn activity(snapshot: ActivitySnapshot, dir: String) -> Self {
        Self::with(Content::Activity(snapshot), dir)
    }
    fn with(content: Content, dir: String) -> Self {
        let mut state = Self {
            content,
            format: Format::Csv,
            table: Table::Contributors,
            emails: false,
            dir,
            name: String::new(),
            stopping: false,
            error: None,
            notice: None,
            saved: None,
            saving: None,
            task: None,
        };
        state.name = state.default_name();
        state
    }
    /// The pull requests the review holds, for a pull request export.
    pub fn pulls(&self) -> Option<&Snapshot> {
        match &self.content {
            Content::Pulls(s) => Some(s.as_ref()),
            Content::Activity(_) => None,
        }
    }
    /// The rows the file holds: pull requests, contributors or weeks.
    pub fn rows(&self) -> usize {
        match &self.content {
            Content::Pulls(s) => s.items.len(),
            Content::Activity(a) if self.format == Format::Csv && self.table == Table::Weeks => a.week_count(),
            Content::Activity(a) => a.activity.authors.len(),
        }
    }
    /// The file name offered for the content, format and table chosen.
    fn default_name(&self) -> String {
        match (&self.content, self.format, self.table) {
            (Content::Pulls(_), format, _) => format!("rocket-insights.{}", format.extension()),
            (Content::Activity(_), Format::Json, _) => "rocket-activity.json".into(),
            (Content::Activity(_), Format::Csv, Table::Contributors) => "rocket-activity-contributors.csv".into(),
            (Content::Activity(_), Format::Csv, Table::Weeks) => "rocket-activity-weeks.csv".into(),
        }
    }
    /// Choose a format or table: an offered name follows it; a typed one keeps its stem.
    fn choose(&mut self, format: Format, table: Table) {
        let offered = self.name == self.default_name();
        (self.format, self.table) = (format, table);
        self.name = if offered {
            self.default_name()
        } else {
            Path::new(&self.name).with_extension(format.extension()).display().to_string()
        };
        self.error = None;
        self.notice = None;
        self.saved = None;
    }
    pub fn running(&self) -> bool {
        self.task.is_some()
    }
    pub fn cancel(&mut self) {
        if let Some(task) = &self.task {
            self.stopping = true;
            task.cancel();
        }
    }
    fn save(&mut self, ctx: &Context, scope: Option<PathBuf>, log: DiagnosticLog) {
        if self.running() {
            return;
        }
        self.error = None;
        self.notice = None;
        self.saved = None;
        self.stopping = false;
        let name = self.name.trim();
        if name.is_empty()
            || name.contains(['/', '\\', ':'])
            || Path::new(name)
                .extension()
                .is_none_or(|e| !e.eq_ignore_ascii_case(self.format.extension()))
        {
            self.error =
                Some(tr("Use a new file name with the selected CSV or JSON extension.").into());
            return;
        }
        let path = Path::new(self.dir.trim()).join(name);
        self.saving = Some(name.to_owned());
        let sample = self.content.clone();
        let (format, table, emails) = (self.format, self.table, self.emails);
        let wake = ctx.clone();
        match WriteTask::spawn(move |cancel| {
            let result = log.request("insights-export", |_| {
                let bytes = sample.encode(format, table, emails, &cancel)?;
                save_before_publish(&path, &bytes, scope.as_deref(), &cancel, || {})
            });
            wake.request_repaint();
            result
        }) {
            Ok(task) => self.task = Some(task),
            Err(e) => self.error = Some(user_error(&e)),
        }
    }
    pub(super) fn poll(&mut self) {
        let Some(result) = self.task.as_mut().and_then(WriteTask::try_finish) else {
            return;
        };
        self.task = None;
        self.stopping = false;
        let name = self.saving.take();
        // Once the new file is published, success wins a late Stop. Never report a
        // cancellation if the reviewed file was already created.
        match result {
            Ok(()) => {
                self.saved = name;
                self.notice = Some(tr("Export saved. Existing files were not replaced.").into())
            }
            Err(Error::Cancelled) => {
                self.notice = Some(tr("Stopped. No export file was created.").into())
            }
            Err(Error::Io { source, .. }) if source.kind() == std::io::ErrorKind::AlreadyExists => {
                self.error = Some(
                    tr("That file already exists. Choose another name; nothing was replaced.")
                        .into(),
                )
            }
            Err(e) => self.error = Some(user_error(&e)),
        }
    }
}

fn save_before_publish(
    path: &Path,
    bytes: &[u8],
    scope: Option<&Path>,
    cancel: &CancellationToken,
    before: impl FnOnce(),
) -> Result<()> {
    cancel.check()?;
    if !path.is_absolute()
        || path
            .components()
            .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
        || path
            .file_name()
            .is_none_or(|n| n.to_string_lossy().contains(':'))
        || path
            .extension()
            .is_none_or(|e| !e.eq_ignore_ascii_case("csv") && !e.eq_ignore_ascii_case("json"))
    {
        return Err(Error::InvalidInput(
            "Choose an absolute path to a new CSV/JSON file in an existing folder.".into(),
        ));
    }
    let parent = path
        .parent()
        .ok_or_else(|| Error::InvalidInput("Choose an export folder.".into()))?;
    let _guards = scope
        .map(|root| crate::folder_browser::scope::pin_directory(parent, root))
        .transpose()?;
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let temporary = parent.join(format!(
        ".rocket-insights-{}-{}.tmp",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let io = |source| Error::Io {
        action: "save Insights to a new file (existing files are never overwritten)",
        source,
    };
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(1);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temporary).map_err(io)?;
    let result = (|| {
        for chunk in bytes.chunks(16384) {
            cancel.check()?;
            file.write_all(chunk).map_err(io)?;
        }
        file.sync_all().map_err(io)?;
        before();
        cancel.check()?;
        fs::hard_link(&temporary, path).map_err(io)?;
        Ok(())
    })();
    drop(file);
    let cleanup = fs::remove_file(&temporary);
    result?;
    cleanup.map_err(|_|Error::Unsupported("Export was saved, but its temporary link could not be removed. Do not retry to the same filename.".into()))
}

/// The width of the labels beside the facts and the fields.
const LABEL: f32 = 112.0;

/// One row of the form: its label in the facts' column, then its control.
fn form_row(ui: &mut Ui, label: &str, content: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 12.0;
        let (rect, _) = ui.allocate_exact_size(vec2(LABEL, CONTROL), Sense::hover());
        ui.painter().text(
            rect.left_center(),
            Align2::LEFT_CENTER,
            label,
            FontId::proportional(13.0),
            HINT(),
        );
        content(ui);
    });
}

pub(super) fn dialog(
    ctx: &Context,
    state: &mut Option<State>,
    widgets: &mut HashMap<String, UiControl>,
    backend: Option<&Backend>,
    scope: Option<PathBuf>,
) -> Option<String> {
    let Some(e) = state.as_mut() else { return None };
    let mut browse = None;
    let mut save = false;
    // The title, the line under it, and what the file holds, for each kind of export.
    let (title, subtitle, facts, warning) = match &e.content {
        Content::Pulls(snapshot) => {
            let listed = tr(match snapshot.state {
                "open" => mark("Open"),
                "merged" => mark("Merged"),
                "waiting" => mark("Waiting"),
                _ => mark("All states"),
            });
            let author = snapshot
                .author
                .clone()
                .unwrap_or_else(|| tr("All authors").to_owned());
            let reviewer = snapshot
                .reviewer
                .as_ref()
                .map_or_else(|| tr("All reviewers").to_owned(), |w| w.name.clone());
            let rows = snapshot.items.len();
            let subtitle = format!(
                "{} · {author} · {reviewer} · {listed}",
                trn!(rows, "{n} pull request", "{n} pull requests")
            );
            let r = &snapshot.review_metrics;
            let reviews = if !snapshot.times_reviews {
                tr("Not measured on this host").to_owned()
            } else if r.opened == 0 {
                tr("None opened in the summary period").to_owned()
            } else {
                trf!(
                    "Read for {read} of {opened} opened in the summary period",
                    read = r.read,
                    opened = r.opened
                )
            };
            let facts = vec![
                (tr("Rows"), trn!(rows, "{n} pull request, as listed", "{n} pull requests, as listed"), false),
                (tr("Filters"), format!("{author} · {reviewer} · {listed}"), false),
                (tr("Reviews"), reviews, false),
                (
                    tr("Summary period"),
                    Period::ALL
                        .iter()
                        .find(|p| p.key() == snapshot.period)
                        .unwrap_or(&Period::Days30)
                        .label()
                        .to_owned(),
                    false,
                ),
                (
                    tr("Loaded sample"),
                    trn!(
                        snapshot.loaded,
                        "{n} loaded from {repo} · read {date} UTC",
                        "{n} loaded from {repo} · read {date} UTC",
                        repo = snapshot.repository,
                        date = date(snapshot.read_at as i64, 0)
                    ),
                    snapshot.partial,
                ),
            ];
            (
                tr("Export pull requests"),
                subtitle,
                facts,
                tr("Holds repository and pull request details. Check it before you share it."),
            )
        }
        Content::Activity(a) => {
            let commits = trn!(a.activity.commits, "{n} commit", "{n} commits");
            let contributors = trn!(a.activity.authors.len(), "{n} contributor", "{n} contributors");
            let branches = if a.scope.1.all_branches { tr("All branches") } else { tr("This branch") };
            let scope_line = format!("{branches} · {}", a.scope.0.words());
            let weeks = a.week_count();
            let rows = match (e.format, e.table) {
                (Format::Csv, Table::Contributors) => trn!(
                    a.activity.authors.len(),
                    "{n} contributor, most commits first",
                    "{n} contributors, most commits first"
                ),
                (Format::Csv, Table::Weeks) => {
                    trn!(weeks, "{n} week, empty weeks included", "{n} weeks, empty weeks included")
                }
                (Format::Json, _) => format!(
                    "{contributors} · {} · {}",
                    trn!(weeks, "{n} week", "{n} weeks"),
                    tr("weekdays and hours")
                ),
            };
            let (when, _) = super::refreshed(a.read_at, ctx);
            let facts = vec![
                (tr("Rows"), rows, false),
                (tr("Scope"), scope_line.clone(), false),
                (
                    tr("Counted"),
                    trf!("{commits}, names after .mailmap · read {when}", commits, when),
                    a.activity.limited,
                ),
            ];
            let warning = if e.format == Format::Csv && e.table == Table::Weeks {
                tr("Holds repository activity. Check it before you share it.")
            } else if e.emails {
                tr("Holds contributor names and email addresses. Check it before you share it.")
            } else {
                tr("Holds contributor names. Check it before you share it.")
            };
            (
                tr("Export commit activity"),
                format!("{commits} · {contributors} · {scope_line}"),
                facts,
                warning,
            )
        }
    };
    let activity = matches!(e.content, Content::Activity(_));
    let response = pages::review_at(
        ctx,
        "insights-export-dialog",
        640.0,
        ReviewHeader {
            image: Icon::Activity,
            title,
            subtitle: &subtitle,
            danger: false,
        },
        widgets,
        "insights-export-body",
        true,
        |ui, part, widgets| match part {
            Part::Body => {
                // What the file will hold, as it was when Export was chosen.
                pages::well(ui, |ui| {
                    for (key, value, partial) in &facts {
                        pages::key_value(ui, key, LABEL, |ui| {
                            if *partial {
                                // A row starts as tall as a control; this one is a line
                                // of text, level with its label.
                                ui.spacing_mut().interact_size.y = 18.0;
                                ui.horizontal_wrapped(|ui| {
                                    ui.spacing_mut().item_spacing.x = 8.0;
                                    ui.label(value);
                                    let pill = if activity {
                                        trf!("Newest {count} only", count = ACTIVITY_COMMITS)
                                    } else {
                                        tr("Partial").to_owned()
                                    };
                                    pages::pill(ui, &pill, AMBER());
                                });
                            } else {
                                ui.add(Label::new(value).wrap());
                            }
                        });
                    }
                });
                ui.add_space(4.0);
                ui.add_enabled_ui(!e.running(), |ui| {
                    form_row(ui, tr("Format"), |ui| {
                        let options = [
                            ("CSV", e.format == Format::Csv),
                            ("JSON", e.format == Format::Json),
                        ];
                        for (r, format) in pages::segments(ui, &options)
                            .iter()
                            .zip([Format::Csv, Format::Json])
                        {
                            pages::control(
                                widgets,
                                format!("insights-export-{}", format.extension()),
                                r,
                                ui.clip_rect(),
                            );
                            if r.clicked() && e.format != format {
                                e.choose(format, e.table);
                            }
                        }
                        let hint = match (activity, e.format) {
                            (false, Format::Csv) => mark("For spreadsheets. Formula-like text gets a leading apostrophe."),
                            (false, Format::Json) => mark("Exact text, with the summary numbers."),
                            (true, Format::Csv) => mark("For spreadsheets. One table a file."),
                            (true, Format::Json) => mark("Exact text, with the totals and what was counted."),
                        };
                        ui.add(Label::new(RichText::new(tr(hint)).size(12.0).color(FAINT())).truncate());
                    });
                    if activity {
                        form_row(ui, tr("Table"), |ui| {
                            if e.format == Format::Json {
                                ui.label(tr("Every table: totals, contributors, weeks, weekdays and hours"));
                                return;
                            }
                            let options = [
                                (tr("Contributors"), e.table == Table::Contributors),
                                (tr("Commits per week"), e.table == Table::Weeks),
                            ];
                            for (r, (table, key)) in pages::segments(ui, &options).iter().zip([
                                (Table::Contributors, "insights-export-table-contributors"),
                                (Table::Weeks, "insights-export-table-weeks"),
                            ]) {
                                pages::control(widgets, key, r, ui.clip_rect());
                                if r.clicked() && e.table != table {
                                    e.choose(e.format, table);
                                }
                            }
                        });
                        form_row(ui, tr("Email addresses"), |ui| {
                            let weeks = e.format == Format::Csv && e.table == Table::Weeks;
                            let label = if weeks { tr("Not in this table") } else { tr("Include email addresses") };
                            let mut on = e.emails && !weeks;
                            let r = ui.add_enabled(!weeks, Checkbox::new(&mut on, label));
                            pages::control(widgets, "insights-export-emails", &r, ui.clip_rect());
                            if r.changed() {
                                e.emails = on;
                                e.notice = None;
                                e.saved = None;
                            }
                        });
                    }
                    form_row(ui, tr("Save in"), |ui| {
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let r = ui.add(Button::new(tr("Browse")));
                            pages::control(widgets, "insights-export-browse", &r, ui.clip_rect());
                            if r.clicked() {
                                browse = Some(e.dir.clone());
                            }
                            let r = ui.add_sized(
                                [ui.available_width(), CONTROL],
                                TextEdit::singleline(&mut e.dir)
                                    .id_salt("insights-export-folder")
                                    .char_limit(4096)
                                    .align(Align2::LEFT_CENTER),
                            );
                            pages::control(widgets, "insights-export-folder", &r, ui.clip_rect());
                        });
                    });
                    form_row(ui, tr("File name"), |ui| {
                        let r = ui.add_sized(
                            [ui.available_width(), CONTROL],
                            TextEdit::singleline(&mut e.name)
                                .id_salt("insights-export-name")
                                .char_limit(255)
                                .align(Align2::LEFT_CENTER),
                        );
                        pages::control(widgets, "insights-export-name", &r, ui.clip_rect());
                    });
                });
                // A refused name or folder is said where it was typed.
                if let Some(error) = &e.error {
                    ui.horizontal(|ui| {
                        ui.add_space(LABEL + 12.0);
                        ui.add(Label::new(RichText::new(error).size(12.5).color(RED())).wrap());
                    });
                }
                ui.add_space(2.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    let (mark, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
                    icon(ui.painter(), mark, Icon::Warning, AMBER());
                    ui.add(Label::new(RichText::new(warning).size(12.5).color(AMBER())).wrap());
                });
            }
            Part::Footer => {
                let r = pages::primary(ui, !e.running() && backend.is_some(), tr("Save"));
                pages::control(widgets, "insights-export-save", &r, ui.clip_rect());
                save = r.clicked();
                if e.running() {
                    let r = pages::ghost(
                        ui,
                        !e.stopping,
                        tr(if e.stopping { "Stopping…" } else { "Stop" }),
                    );
                    pages::control(widgets, "insights-export-stop", &r, ui.clip_rect());
                    if r.clicked() {
                        e.cancel();
                    }
                    pages::footer_status(ui, true, tr("Saving…"));
                } else if let Some(name) = &e.saved {
                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        let (mark, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
                        icon(ui.painter(), mark, Icon::Check, GREEN());
                        ui.add(
                            Label::new(
                                RichText::new(trf!("Saved {name}", name))
                                    .size(12.5)
                                    .color(GREEN()),
                            )
                            .truncate(),
                        );
                    });
                } else {
                    pages::footer_status(
                        ui,
                        false,
                        e.notice
                            .as_deref()
                            .unwrap_or(tr("Saves a new file. Never replaces one.")),
                    );
                }
            }
        },
    );
    if let Some(close) = widgets.get("dialog-close").copied() {
        widgets.insert("insights-export-close".into(), close);
    }
    if save {
        if let Some(backend) = backend {
            e.save(ctx, scope, backend.log.clone());
        }
    }
    if response.should_close() {
        if e.running() {
            e.cancel();
        } else {
            *state = None;
        }
    }
    browse
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn csv_quotes_unicode_and_neutralizes_formula_prefixes_without_changing_json() {
        for value in [
            "=1+1",
            "+SUM(1,2)",
            "-2+4",
            "@SUM(A1)",
            "  =1",
            "\t+1",
            "\rhello",
            "\ntext",
            "\u{feff}=9",
            "\u{200b}=9",
        ] {
            assert!(csv_cell(value).starts_with("\"'"), "{value:?}");
        }
        assert_eq!(
            csv_cell("資料, \"quoted\"\nnext"),
            "\"資料, \"\"quoted\"\"\nnext\""
        );
        let s = sample();
        let json = s.json();
        assert_eq!(json["items"][0]["title"], "=1+1");
        assert!(json["items"][0]["created_unix_seconds"].is_null());
        assert_eq!(json["filters"]["rows_period"], "all_loaded_dates");
        assert_eq!(json["coverage"]["loaded_unique_prs"], 7);
        assert_eq!(json["coverage"]["exported_rows"], 1);
        assert_eq!(json["metrics"]["loaded"], 3);
        let text =
            String::from_utf8(s.encode(Format::Csv, &CancellationToken::new()).unwrap()).unwrap();
        assert!(text.starts_with('\u{feff}'));
        assert!(text.contains("\"'=1+1\""));
        assert!(!text.contains("NOT_EXPORTED_URL"));
        assert!(!json.to_string().contains("NOT_EXPORTED_URL"));
    }
    /// The review columns come after every earlier column, a reviewer named like a
    /// formula is neutralized, and each row's reviews say in a word what is known.
    #[test]
    fn insights_export_appends_review_columns_and_names_missing_reviews() {
        let mut s = sample();
        // Without a reviewer the scope keeps the word earlier exports used.
        assert_eq!(s.json()["metrics"]["scope"], "author_filtered_loaded_sample_before_state_filter");
        let second = ForgeItem { number: 43, reviewers: None, ..s.items[0].clone() };
        // Merged with a request left over: nobody waits on it.
        let mut merged = ForgeItem { number: 44, ..s.items[0].clone() };
        merged.timing = Some(ForgePullTiming { is_merged: true, ..Default::default() });
        // Reviewers named without who still waits (GitLab, Gitea).
        let mut assigned = ForgeItem { number: 45, ..s.items[0].clone() };
        assigned.reviewers = Some(ForgeReviewRequests {
            assigned: vec![ForgeReviewer { team: false, name: "Cy".into() }],
            ..Default::default()
        });
        s.items.extend([second, merged, assigned]);
        s.reviews.extend([None, None, None]);
        s.reviewer = Some(ForgeReviewer { team: false, name: "=Bob".into() });
        assert_eq!(s.json()["metrics"]["scope"], "author_and_reviewer_filtered_loaded_sample_before_state_filter");
        let text =
            String::from_utf8(s.encode(Format::Csv, &CancellationToken::new()).unwrap()).unwrap();
        let mut lines = text.trim_start_matches('\u{feff}').lines();
        let header: Vec<&str> = lines.next().unwrap().split(',').collect();
        assert_eq!(
            header[header.len() - 6..],
            ["merged", "reviewer_filter", "review_requests", "reviews", "first_review_unix_seconds", "first_reviewer"]
        );
        let first = lines.next().unwrap();
        assert!(first.ends_with("\"'=Bob\",\"'=Bob; team/core\",\"read\",\"190\",\"Ann\""), "{first}");
        let second = lines.next().unwrap();
        assert!(second.ends_with("\"'=Bob\",\"not_in_list\",\"not_read\",\"\",\"\""), "{second}");
        let merged = lines.next().unwrap();
        assert!(merged.ends_with("\"'=Bob\",\"\",\"not_read\",\"\",\"\""), "{merged}");
        let assigned = lines.next().unwrap();
        assert!(assigned.ends_with("\"'=Bob\",\"no_review_state\",\"not_read\",\"\",\"\""), "{assigned}");
        let json = s.json();
        assert_eq!(json["filters"]["reviewer"]["name"], "=Bob");
        assert_eq!(json["items"][0]["review_requests"][1]["team"], true);
        assert_eq!(json["items"][0]["first_review_unix_seconds"], 190);
        assert!(json["items"][1]["review_requests"].is_null());
        assert_eq!(json["items"][1]["review_requests_unknown"], "not_in_list");
        assert_eq!(json["items"][1]["reviews"], "not_read");
        assert_eq!(json["items"][2]["review_requests"], serde_json::json!([]));
        assert!(json["items"][2]["review_requests_unknown"].is_null());
        assert_eq!(json["items"][3]["review_requests_unknown"], "no_review_state");
        assert_eq!(json["metrics"]["first_review"]["measured"], true);
        // A host without review times says so for every row.
        s.times_reviews = false;
        assert_eq!(s.json()["items"][0]["reviews"], "not_measured");
        // A read that could not tell the first review leaves the time out.
        s.times_reviews = true;
        s.reviews[0] = Some(ForgeReviewRead::Read(ForgeFirstReview {
            first: Some(("Ann".into(), 190)),
            undated: true,
            ..Default::default()
        }));
        let json = s.json();
        assert_eq!(json["items"][0]["reviews"], "no_usable_time");
        assert!(json["items"][0]["first_review_unix_seconds"].is_null());
    }
    fn sample() -> Snapshot {
        Snapshot {
            items: vec![ForgeItem {
                number: 42,
                title: "=1+1".into(),
                author: "作者".into(),
                state: "open".into(),
                source: "資料".into(),
                target: "main".into(),
                url: "NOT_EXPORTED_URL".into(),
                // Open, without dates.
                timing: Some(ForgePullTiming { open: true, ..Default::default() }),
                reviewers: Some(ForgeReviewRequests {
                    known: true,
                    waiting: vec![
                        ForgeReviewer { team: false, name: "=Bob".into() },
                        ForgeReviewer { team: true, name: "team/core".into() },
                    ],
                    ..Default::default()
                }),
            }],
            reviewer: None,
            review_metrics: ForgeReviewMetrics::default(),
            times_reviews: true,
            reviews: vec![Some(ForgeReviewRead::Read(ForgeFirstReview {
                first: Some(("Ann".into(), 190)),
                reviewers: vec!["Ann".into()],
                ..Default::default()
            }))],
            repository: "example/資料".into(),
            provider: "github".into(),
            server: "https://github.com".into(),
            read_at: 123,
            pages: 1,
            loaded: 7,
            partial: true,
            author: Some("作者".into()),
            state: "open",
            period: "30",
            metrics: ForgePullMetrics {
                loaded: 3,
                ..Default::default()
            },
        }
    }
    #[test]
    fn insights_export_publication_preserves_existing_racing_and_cancelled_files() {
        let root = std::env::temp_dir().join(format!(
            "rocket-insights-export-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        let path = root.join("資料 🚀.json");
        let c = CancellationToken::new();
        let bytes = sample().encode(Format::Json, &c).unwrap();
        save_before_publish(&path, &bytes, Some(&root), &c, || {}).unwrap();
        assert!(save_before_publish(&path, b"replacement", Some(&root), &c, || {}).is_err());
        assert_eq!(fs::read(&path).unwrap(), bytes);
        let race = root.join("race.csv");
        assert!(
            save_before_publish(&race, b"candidate", Some(&root), &c, || fs::write(
                &race,
                b"external"
            )
            .unwrap())
            .is_err()
        );
        assert_eq!(fs::read(&race).unwrap(), b"external");
        let stopped = root.join("stopped.csv");
        let cancel = CancellationToken::new();
        assert!(matches!(
            save_before_publish(&stopped, b"candidate", Some(&root), &cancel, || cancel
                .cancel()),
            Err(Error::Cancelled)
        ));
        assert!(!stopped.exists());
        assert!(sample().encode(Format::Json, &cancel).is_err());
        assert!(
            save_before_publish(&root.join("../outside.json"), b"x", Some(&root), &c, || {})
                .is_err()
        );
        assert!(
            save_before_publish(&root.join("wrong.exe"), b"x", Some(&root), &c, || {}).is_err()
        );
        assert!(fs::read_dir(&root).unwrap().all(|e| {
            !e.unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".rocket-insights-")
        }));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn stop_after_export_publication_reports_success_not_a_missing_file() {
        use std::{
            sync::mpsc,
            time::{Duration, Instant},
        };
        let root = std::env::temp_dir().join(format!(
            "rocket-insights-published-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        let path = root.join("saved.json");
        let (sent, received) = mpsc::channel();
        let (release, wait) = mpsc::channel();
        let worker_path = path.clone();
        let worker_root = root.clone();
        let mut e = State::new(&pulls::State::default(), root.display().to_string());
        e.task = Some(
            WriteTask::spawn(move |c| {
                save_before_publish(&worker_path, b"{}", Some(&worker_root), &c, || {})?;
                sent.send(()).unwrap();
                wait.recv_timeout(Duration::from_secs(5)).unwrap();
                Ok(())
            })
            .unwrap(),
        );
        received.recv_timeout(Duration::from_secs(5)).unwrap();
        e.cancel();
        release.send(()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while e.running() {
            e.poll();
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        }
        assert!(e.error.is_none());
        assert!(
            e.notice
                .as_ref()
                .is_some_and(|s| s.contains("Export saved"))
        );
        assert_eq!(fs::read(&path).unwrap(), b"{}");
        fs::remove_dir_all(root).unwrap();
    }

    fn activity() -> ActivitySnapshot {
        let author = |name: &str, email: &str, commits| AuthorActivity {
            name: name.as_bytes().to_vec(),
            email: email.as_bytes().to_vec(),
            commits,
        };
        ActivitySnapshot {
            activity: Arc::new(CommitActivity {
                commits: 3,
                merges: 1,
                first: Some(1_705_307_400),
                last: Some(1_706_745_600),
                authors: vec![author("作者", "author@example.invalid", 2), author("=cmd|' /C calc'!A0", "-x@example.invalid", 1)],
                months: vec![(2024, 1, 3)],
                weeks: vec![(19_737, 2), (19_751, 1)],
                days: 3,
                ..Default::default()
            }),
            repository: "資料".into(),
            scope: (Period::Days90, ActivityScope { all_branches: true, since: Some(1_700_000_000) }),
            read_at: 1_707_000_000,
        }
    }

    #[test]
    fn activity_exports_one_table_a_csv_every_table_in_json_and_addresses_only_when_asked() {
        let (a, c) = (activity(), CancellationToken::new());
        let text = |format, table, emails| {
            String::from_utf8(a.encode(format, table, emails, &c).unwrap()).unwrap()
        };
        let csv = text(Format::Csv, Table::Contributors, false);
        assert!(csv.starts_with('\u{feff}'));
        let head = csv.lines().next().unwrap();
        assert!(head.ends_with(",commits_counted,limited,name,commits"), "{head}");
        assert!(!csv.contains("example.invalid"), "addresses only when ticked");
        // Each row repeats the scope; names stay, formula text is neutralized.
        assert!(csv.contains("\"1\",\"資料\",\"all_branches\",\"90\",\"1700000000\",\"1707000000\",\"3\",\"false\",\"作者\",\"2\""));
        assert!(csv.contains("\"'=cmd|' /C calc'!A0\""));
        let with = text(Format::Csv, Table::Contributors, true);
        assert!(with.lines().next().unwrap().ends_with("name,email,commits"));
        assert!(with.contains("\"'-x@example.invalid\""));
        // Weeks: from the first to the last, the empty one between included.
        let weeks = text(Format::Csv, Table::Weeks, true);
        let rows: Vec<_> = weeks.lines().skip(1).map(|l| l.rsplitn(3, ',').take(2).collect::<Vec<_>>()).collect();
        assert_eq!(rows, [["\"2\"", "\"2024-01-15\""], ["\"0\"", "\"2024-01-22\""], ["\"1\"", "\"2024-01-29\""]]);
        assert!(!weeks.contains("example.invalid"));
        // JSON: exact text and every table; addresses only when ticked.
        let json: Value = serde_json::from_str(&text(Format::Json, Table::Weeks, false)).unwrap();
        assert_eq!(json["kind"], "commit_activity");
        assert_eq!(json["scope"]["branches"], "all_branches");
        assert_eq!(json["scope"]["email_addresses"], false);
        assert_eq!(json["contributors"][1]["name"], "=cmd|' /C calc'!A0");
        assert!(json["contributors"][0].get("email").is_none());
        assert_eq!(json["weeks"].as_array().unwrap().len(), 3);
        assert_eq!(json["weekday_hours"].as_array().unwrap().len(), 7);
        assert_eq!(json["coverage"]["commit_limit"], ACTIVITY_COMMITS);
        let json: Value = serde_json::from_str(&text(Format::Json, Table::Contributors, true)).unwrap();
        assert_eq!(json["contributors"][0]["email"], "author@example.invalid");
        // Stopped before encoding: nothing is produced.
        let stopped = CancellationToken::new();
        stopped.cancel();
        assert!(a.encode(Format::Csv, Table::Weeks, false, &stopped).is_err());
    }

    #[test]
    fn activity_weeks_are_counted_without_listing_them_and_a_huge_span_is_refused() {
        // A commit dated a few thousand years on: far too many weeks for any export.
        let mut a = activity();
        Arc::make_mut(&mut a.activity).weeks = vec![(19_737, 1), (19_737 + 7 * 300_000, 1)];
        assert_eq!(a.week_count(), 300_001);
        let c = CancellationToken::new();
        for format in [Format::Csv, Format::Json] {
            let e = a.encode(format, Table::Weeks, false, &c).unwrap_err();
            assert!(matches!(e, Error::InvalidInput(ref m) if m.contains("16 MiB")), "{format:?}");
        }
        // Contributors don't list weeks, so that table still saves.
        assert!(a.encode(Format::Csv, Table::Contributors, false, &c).is_ok());
        // The review counts the rows without building them.
        let mut e = State::activity(a, String::new());
        e.choose(Format::Csv, Table::Weeks);
        assert_eq!(e.rows(), 300_001);
        assert_eq!(activity().week_count(), 3);
        let mut none = activity();
        Arc::make_mut(&mut none.activity).weeks.clear();
        assert_eq!((none.week_count(), none.weeks(&c).unwrap().len()), (0, 0));
    }

    #[test]
    fn activity_export_offers_a_name_per_table_and_keeps_a_typed_one() {
        let mut e = State::activity(activity(), String::new());
        assert_eq!((e.name.as_str(), e.rows(), e.emails), ("rocket-activity-contributors.csv", 2, false));
        e.choose(Format::Csv, Table::Weeks);
        assert_eq!((e.name.as_str(), e.rows()), ("rocket-activity-weeks.csv", 3));
        e.choose(Format::Json, Table::Weeks);
        assert_eq!(e.name, "rocket-activity.json");
        e.name = "mine.json".into();
        e.choose(Format::Csv, Table::Weeks);
        assert_eq!(e.name, "mine.csv");
        assert!(e.pulls().is_none());
        // A pull request export keeps its name and holds the rows as listed.
        let p = State::new(&pulls::State::default(), String::new());
        assert_eq!((p.name.as_str(), p.rows()), ("rocket-insights.csv", 0));
    }
}
