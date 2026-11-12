//! Repository Statistics: commit activity (authors, months, weekdays and hours) and an
//! activity treemap of the files at HEAD. Both are read on workers and never write.
use crate::{
    app::{DialogKind, Rocket, UiControl},
    backend::user_error,
    pages,
    style::*,
};
use eframe::egui::*;
use rocket_git::*;
use std::{
    collections::HashMap,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
pub mod pulls;
pub mod export;
mod pull_detail;
mod trends;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Section {
    #[default]
    Activity,
    Pulls,
    Treemap,
}
impl Section {
    pub const ALL: [Self; 3] = [Self::Activity, Self::Pulls, Self::Treemap];
}

/// How far back the counted commits go, from now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Period {
    Days30,
    Days90,
    Year,
    All,
}
impl Period {
    pub const ALL: [Self; 4] = [Self::Days30, Self::Days90, Self::Year, Self::All];
    /// The period in a sentence: "in the last 30 days", "in the last year".
    pub fn phrase(self) -> &'static str {
        match self {
            Self::Days30 => crate::i18n::tr("in the last 30 days"),
            Self::Days90 => crate::i18n::tr("in the last 90 days"),
            Self::Year => crate::i18n::tr("in the last year"),
            Self::All => crate::i18n::tr("in all history"),
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Days30 => crate::i18n::tr("30 days"),
            Self::Days90 => crate::i18n::tr("90 days"),
            Self::Year => crate::i18n::tr("1 year"),
            Self::All => crate::i18n::tr("All"),
        }
    }
    /// The period before this one, as a heading: "Previous 30 days".
    pub fn previous_label(self) -> &'static str {
        match self {
            Self::Days30 => crate::i18n::tr("Previous 30 days"),
            Self::Days90 => crate::i18n::tr("Previous 90 days"),
            Self::Year | Self::All => crate::i18n::tr("Previous year"),
        }
    }
    /// This period as a heading: "Last 30 days".
    pub fn last_label(self) -> &'static str {
        match self {
            Self::Days30 => crate::i18n::tr("Last 30 days"),
            Self::Days90 => crate::i18n::tr("Last 90 days"),
            Self::Year => crate::i18n::tr("Last year"),
            Self::All => crate::i18n::tr("All history"),
        }
    }
    /// The comparison card's title.
    pub fn compared_title(self) -> &'static str {
        match self {
            Self::Days30 => crate::i18n::tr("Compared with the previous 30 days"),
            Self::Days90 => crate::i18n::tr("Compared with the previous 90 days"),
            Self::Year | Self::All => crate::i18n::tr("Compared with the previous year"),
        }
    }
    /// The period in a few words: "last 30 days", "all history".
    pub fn words(self) -> &'static str {
        match self {
            Self::Days30 => crate::i18n::tr("last 30 days"),
            Self::Days90 => crate::i18n::tr("last 90 days"),
            Self::Year => crate::i18n::tr("last year"),
            Self::All => crate::i18n::tr("all history"),
        }
    }
    /// The automation key: days, or `all`.
    pub fn key(self) -> &'static str {
        match self {
            Self::Days30 => "30",
            Self::Days90 => "90",
            Self::Year => "365",
            Self::All => "all",
        }
    }
    /// The oldest counted time (Unix seconds), or `None` for all history.
    pub fn since(self) -> Option<i64> {
        let days = match self {
            Self::Days30 => 30,
            Self::Days90 => 90,
            Self::Year => 365,
            Self::All => return None,
        };
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() as i64);
        Some(now - days * 86_400)
    }
}

const STOPPED: &str = crate::i18n::mark("Stopped. Choose Refresh to read again.");
/// The treemap draws at most this many entries of a folder; smaller ones are combined.
const CELLS: usize = 400;

pub struct State {
    pub section: Section,
    /// Commit activity: how far back, and whether every branch counts.
    pub range: Period,
    pub all_branches: bool,
    /// Treemap: the period whose changes colour the files.
    pub period: Period,
    pub activity: Option<Arc<CommitActivity>>,
    pub treemap: Option<Arc<ActivityTreemap>>,
    /// Folder names from the treemap's root to the folder shown.
    pub zoom: Vec<Vec<u8>>,
    pub activity_error: Option<String>,
    pub treemap_error: Option<String>,
    /// Tables instead of charts.
    pub table: bool,
    pub pulls: pulls::State,
    /// When the activity and the treemap shown were read (Unix seconds).
    pub activity_at: u64,
    pub treemap_at: u64,
    /// The reviewed export, of pull requests or of commit activity.
    pub export: Option<export::State>,
    activity_task: Option<ReadTask<CommitActivity>>,
    treemap_task: Option<ReadTask<ActivityTreemap>>,
    /// The choices the activity and the treemap shown were read for, and the ones a
    /// running read is for, so a read stopped by leaving the page is never shown under
    /// choices it wasn't read for.
    activity_for: Option<(Period, ActivityScope)>,
    activity_asked: (Period, ActivityScope),
    treemap_for: Option<Period>,
    treemap_asked: Period,
    /// The repository's [`History`] each was read at, so numbers read before HEAD or a
    /// branch moved are read again rather than shown as current.
    activity_history: Option<u64>,
    activity_history_asked: Option<u64>,
    treemap_history: Option<u64>,
    treemap_history_asked: Option<u64>,
    /// The shown folder's most changed files, for the treemap and zoom they were
    /// worked out for.
    top: Option<TopFiles>,
}
type TopFiles = ((usize, Vec<Vec<u8>>), Vec<(String, usize, u64)>);

/// What the numbers depend on besides the choices: HEAD's commit (this branch, and the
/// files at HEAD), and every branch tip with it (all branches). Fingerprints from the
/// tab's last status read; `None` before the first one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct History {
    head: u64,
    all: u64,
}
impl History {
    pub(crate) fn of(snapshot: Option<&crate::backend::Snapshot>) -> Option<Self> {
        use std::hash::{DefaultHasher, Hash, Hasher};
        let snapshot = snapshot?;
        let mut head = DefaultHasher::new();
        match &snapshot.head {
            Some(Head::Branch { target, .. } | Head::Detached { target }) => target.hash(&mut head),
            Some(Head::Unborn { branch }) => branch.hash(&mut head),
            None => return None,
        }
        let head = head.finish();
        let mut all = DefaultHasher::new();
        head.hash(&mut all);
        for branch in &snapshot.branches {
            (&branch.reference, &branch.target).hash(&mut all);
        }
        Some(Self { head, all: all.finish() })
    }
    fn activity(self, all_branches: bool) -> u64 {
        if all_branches { self.all } else { self.head }
    }
}
impl Default for State {
    fn default() -> Self {
        Self::from_view(&View::default())
    }
}

/// The page's choices for one repository, kept between sessions: the tab, and each
/// tab's filters.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct View {
    pub section: Section,
    /// Activity: how far back, every branch, tables.
    pub range: Period,
    pub all_branches: bool,
    pub table: bool,
    /// Files: the period that colours the files.
    pub period: Period,
    /// Pull requests: the period, the author, the state shown and Trends.
    pub pull_period: Period,
    pub author: Option<String>,
    pub show: u8,
    pub trends: bool,
}
impl Default for View {
    fn default() -> Self {
        Self {
            section: Section::Activity,
            range: Period::All,
            all_branches: false,
            table: false,
            period: Period::Year,
            pull_period: Period::Days30,
            author: None,
            show: 0,
            trends: false,
        }
    }
}
/// The longest author name a saved view keeps, in bytes.
pub const VIEW_AUTHOR: usize = 1024;
impl View {
    pub fn capture(s: &State) -> Self {
        Self {
            section: s.section,
            range: s.range,
            all_branches: s.all_branches,
            table: s.table,
            period: s.period,
            pull_period: s.pulls.period.unwrap_or(Period::Days30),
            author: s.pulls.author.clone(),
            show: s.pulls.show,
            trends: s.pulls.trends,
        }
    }
    pub fn valid(&self) -> bool {
        // 0 all, 1 open, 2 merged, 3 waiting for review.
        self.show <= 3 && self.author.as_ref().is_none_or(|a| a.len() <= VIEW_AUTHOR)
    }
}

/// "4 min ago" for a read at `read_at` (Unix seconds), and "Read at 13:28" for its
/// tooltip; keeps the page drawing so the first stays current.
pub(crate) fn refreshed(read_at: u64, ctx: &Context) -> (String, String) {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    ctx.request_repaint_after(std::time::Duration::from_secs(30));
    let when = ago(now.saturating_sub(read_at), read_at as i64);
    let at = match rocket_git::local_time(read_at.saturating_mul(1000)) {
        Some(t) if now.saturating_sub(read_at) < 86_400 => format!("{:02}:{:02}", t.hour, t.minute),
        Some(t) => format!("{}-{:02}-{:02} {:02}:{:02}", t.year, t.month, t.day, t.hour, t.minute),
        None => pulls::read_stamp(read_at),
    };
    (when, crate::i18n::trf!("Read at {time}", time = at))
}

impl State {
    pub fn from_view(v: &View) -> Self {
        Self {
            section: v.section,
            range: v.range,
            all_branches: v.all_branches,
            period: v.period,
            activity: None,
            treemap: None,
            zoom: Vec::new(),
            activity_error: None,
            treemap_error: None,
            table: v.table,
            pulls: pulls::State::with_filters(v.pull_period, v.author.clone(), v.show, v.trends),
            activity_at: 0,
            treemap_at: 0,
            export: None,
            activity_task: None,
            treemap_task: None,
            activity_for: None,
            activity_asked: (v.range, ActivityScope::default()),
            treemap_for: None,
            treemap_asked: v.period,
            activity_history: None,
            activity_history_asked: None,
            treemap_history: None,
            treemap_history_asked: None,
            top: None,
        }
    }
    /// Drop numbers read before HEAD or, for all branches, a branch tip moved, so they are
    /// read again instead of shown as this branch's or as the files at HEAD.
    pub(crate) fn forget_moved(&mut self, history: Option<History>) {
        let Some(history) = history else { return };
        if self.activity_for.as_ref().is_some_and(|(_, scope)| {
            self.activity_history != Some(history.activity(scope.all_branches))
        }) {
            (self.activity, self.activity_for, self.activity_history) = (None, None, None);
        }
        if self.treemap_for.is_some() && self.treemap_history != Some(history.head) {
            (self.treemap, self.treemap_for, self.treemap_history, self.top) = (None, None, None, None);
        }
    }
    pub fn running(&self) -> bool {
        self.activity_task.is_some()
            || self.treemap_task.is_some()
            || self.pulls.running()
            || self.export.as_ref().is_some_and(|e| e.running())
    }
    pub fn activity_running(&self) -> bool {
        self.activity_task.is_some()
    }
    pub fn treemap_running(&self) -> bool {
        self.treemap_task.is_some()
    }
    pub fn cancel(&mut self) {
        self.pulls.cancel();
        if let Some(e) = &mut self.export {
            e.cancel();
        }
        if let Some(t) = &self.activity_task {
            t.cancel();
        }
        if let Some(t) = &self.treemap_task {
            t.cancel();
        }
    }
    /// The page is no longer shown: stop its reads and its export review (dropping a
    /// task cancels it) and keep the numbers, their filters and when they were read.
    pub(crate) fn park(&mut self) {
        if self.activity_task.is_some()
            || self.treemap_task.is_some()
            || self.export.is_some()
            || self.pulls.running()
            || self.pulls.detail.number.is_some()
        {
            self.activity_task = None;
            self.treemap_task = None;
            self.export = None;
            self.pulls.park();
        }
    }
    /// Whether a section has no numbers for its choices, so showing it reads them.
    fn needs_read(&self, section: Section) -> bool {
        match section {
            Section::Activity => {
                !self.activity_running()
                    && self.activity_for.as_ref().is_none_or(|(range, scope)| {
                        *range != self.range || scope.all_branches != self.all_branches
                    })
            }
            Section::Pulls => !self.pulls.loaded && !self.pulls.list_running(),
            Section::Treemap => !self.treemap_running() && self.treemap_for != Some(self.period),
        }
    }
    /// The treemap folder shown, if the zoom path exists in the loaded treemap.
    pub fn shown(&self) -> Option<&TreemapNode> {
        folder_at(self.treemap.as_ref()?, &self.zoom)
    }
}

/// The folder reached from the treemap's root through `zoom`, if it exists.
fn folder_at<'a>(map: &'a ActivityTreemap, zoom: &[Vec<u8>]) -> Option<&'a TreemapNode> {
    let mut node = &map.root;
    for name in zoom {
        node = node.children.iter().find(|c| c.folder && &c.name == name)?;
    }
    Some(node)
}

impl Rocket {
    /// The tab whose Insights page is shown, while it is.
    fn insights_owner(&self) -> Option<u64> {
        self.dialog
            .as_ref()
            .filter(|d| matches!(d.kind, DialogKind::Statistics))
            .and_then(|d| d.owner)
    }
    /// The shown Insights page's state. Each repository tab keeps its own, so leaving
    /// the page and coming back shows the same numbers and filters.
    pub fn statistics(&self) -> Option<&State> {
        self.insights.get(&self.insights_owner()?)
    }
    pub(crate) fn statistics_mut(&mut self) -> Option<&mut State> {
        let owner = self.insights_owner()?;
        self.insights.get_mut(&owner)
    }
    /// Insights was opened: the tab's state, or one from its saved choices, and a read
    /// only of a section that has no numbers for its choices yet.
    pub(crate) fn open_statistics(&mut self) {
        let Some(owner) = self.insights_owner() else { return };
        if !self.insights.contains_key(&owner) {
            let Some(path) = self.tabs.iter().find(|t| t.id == owner).map(|t| t.path.clone()) else {
                return;
            };
            let view = self.saved_insights_view(&path);
            self.insights.insert(owner, State::from_view(&view));
        }
        let history = History::of(self.tabs.iter().find(|t| t.id == owner).and_then(|t| t.snapshot.as_ref()));
        let Some(s) = self.insights.get_mut(&owner) else { return };
        s.forget_moved(history);
        if s.needs_read(s.section) {
            self.start_statistics_read();
        }
    }
    /// Read the shown section of Insights, replacing an older read of it.
    pub(crate) fn start_statistics_read(&mut self) {
        let Some(owner) = self.insights_owner() else { return };
        let Some(tab) = self.tabs.iter().find(|t| t.id == owner) else { return };
        let path = tab.path.clone();
        let history = History::of(tab.snapshot.as_ref());
        let context = Some(tab)
            .and_then(|t| Some((t.ci.repository.clone()?, t.ci.session()?.clone())));
        let (Some(backend), Some(s)) = (self.backend.clone(), self.insights.get_mut(&owner)) else {
            return;
        };
        match s.section {
            Section::Pulls => {
                s.pulls.start(context, backend, self.context.clone());
            }
            Section::Activity => {
                if let Some(t) = s.activity_task.take() {
                    t.cancel();
                }
                s.activity_error = None;
                let scope = ActivityScope {
                    all_branches: s.all_branches,
                    since: s.range.since(),
                };
                s.activity_asked = (s.range, scope.clone());
                s.activity_history_asked = history.map(|h| h.activity(s.all_branches));
                match ReadTask::spawn(move |cancel| backend.commit_activity(&path, &scope, &cancel))
                {
                    Ok(task) => s.activity_task = Some(task),
                    Err(e) => s.activity_error = Some(user_error(&e)),
                }
            }
            Section::Treemap => {
                if let Some(t) = s.treemap_task.take() {
                    t.cancel();
                }
                s.treemap_error = None;
                s.treemap_asked = s.period;
                s.treemap_history_asked = history.map(|h| h.head);
                let since = s.period.since();
                match ReadTask::spawn(move |cancel| backend.activity_treemap(&path, since, &cancel))
                {
                    Ok(task) => s.treemap_task = Some(task),
                    Err(e) => s.treemap_error = Some(user_error(&e)),
                }
            }
        }
    }
    pub(crate) fn poll_statistics(&mut self) {
        let owner = self.insights_owner();
        // A page that isn't shown keeps its numbers but no reads; a closed tab takes its
        // page along.
        let tabs = &self.tabs;
        self.insights.retain(|id, _| tabs.iter().any(|t| t.id == *id));
        for (id, s) in &mut self.insights {
            if Some(*id) != owner {
                s.park();
            }
        }
        let Some(owner) = owner else { return };
        let context = self
            .tabs
            .iter()
            .find(|t| t.id == owner)
            .and_then(|t| Some((t.ci.repository.clone()?, t.ci.session()?.clone())));
        let Some(s) = self.insights.get_mut(&owner) else {
            return;
        };
        s.pulls.poll(context.as_ref());
        if let Some(e) = &mut s.export {
            e.poll();
        }
        let message = |e: &Error| match e {
            Error::Cancelled => crate::i18n::tr(STOPPED).to_owned(),
            e => user_error(e),
        };
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        if let Some(result) = s.activity_task.as_mut().and_then(ReadTask::try_finish) {
            s.activity_task = None;
            match result {
                Ok(a) => {
                    s.activity = Some(Arc::new(a));
                    s.activity_at = now;
                    s.activity_for = Some(s.activity_asked.clone());
                    s.activity_history = s.activity_history_asked;
                }
                Err(e) => s.activity_error = Some(message(&e)),
            }
        }
        if let Some(result) = s.treemap_task.as_mut().and_then(ReadTask::try_finish) {
            s.treemap_task = None;
            match result {
                Ok(map) => {
                    s.treemap = Some(Arc::new(map));
                    s.treemap_at = now;
                    s.treemap_for = Some(s.treemap_asked);
                    s.treemap_history = s.treemap_history_asked;
                    if s.shown().is_none() {
                        s.zoom.clear();
                    }
                }
                Err(e) => s.treemap_error = Some(message(&e)),
            }
        }
    }
    pub(crate) fn statistics_page(&mut self, ui: &mut Ui) {
        let backend = self.backend.clone();
        let scope = self.submodule_fixture_root.clone();
        let downloads = self.tab().and_then(crate::preview_tab::downloads);
        let owner = self.insights_owner();
        // Shown without passing through opening (its state went with an earlier tab of
        // the same id, say): start it as opening would.
        if owner.is_some_and(|owner| !self.insights.contains_key(&owner)) {
            self.open_statistics();
        }
        let Some(mut s) = owner.and_then(|owner| self.insights.remove(&owner)) else {
            // No repository behind the page: say so, with the way out.
            let widgets = &mut self.frame_widgets;
            let mut close = false;
            pages::header(ui, Icon::Activity, crate::i18n::tr("Insights"), "", |ui| {
                let r = pages::ghost(ui, true, crate::i18n::tr("Close"));
                pages::control(widgets, "dialog-cancel", &r, ui.clip_rect());
                close = r.clicked();
            });
            pages::empty(ui, crate::i18n::tr("Open a repository to see its Insights."));
            if close {
                self.dialog = None;
            }
            return;
        };
        // The repository's folder name, for an activity export.
        let tab = owner.and_then(|owner| self.tabs.iter().find(|t| t.id == owner));
        let history = History::of(tab.and_then(|t| t.snapshot.as_ref()));
        let repository = tab
            .and_then(|t| t.path.file_name().map(|n| n.to_string_lossy().into_owned()))
            .unwrap_or_default();
        let widgets = &mut self.frame_widgets;
        let (mut reload, mut close) = (false, false);
        let (mut hosting, mut export) = (false, None);
        let running = match s.section {
            Section::Activity => s.activity_running(),
            Section::Pulls => s.pulls.running(),
            Section::Treemap => s.treemap_running(),
        };
        pages::header(
            ui,
            Icon::Activity,
            crate::i18n::tr("Insights"),
            crate::i18n::tr("Repository activity and pull requests · read-only"),
            |ui| {
                let r = pages::ghost(ui, true, crate::i18n::tr("Close"));
                pages::control(widgets, "dialog-cancel", &r, ui.clip_rect());
                close = r.clicked();
                let r = pages::ghost(ui, running, crate::i18n::tr("Stop"));
                pages::control(widgets, "stats-cancel", &r, ui.clip_rect());
                if r.clicked() {
                    s.cancel();
                }
                let r = crate::chrome::icon_button(ui, Icon::Refresh, !running, false, 32.0)
                    .on_hover_text(crate::i18n::tr("Read again"));
                pages::control(widgets, "stats-refresh", &r, ui.clip_rect());
                reload |= r.clicked();
                if r.clicked() { s.pulls.want_more = false; }
            },
        );
        let tabs = [
            pages::TabItem {
                label: crate::i18n::tr("Activity"),
                count: None,
                selected: s.section == Section::Activity,
                enabled: true,
                key: "stats-section-activity",
            },
            pages::TabItem {
                label: crate::i18n::tr("Pull requests"),
                count: None,
                selected: s.section == Section::Pulls,
                enabled: true,
                key: "stats-section-pulls",
            },
            pages::TabItem {
                label: crate::i18n::tr("Files"),
                count: None,
                selected: s.section == Section::Treemap,
                enabled: true,
                key: "stats-section-treemap",
            },
        ];
        if let Some(i) = pages::tabs(ui, &tabs, widgets) {
            s.section = Section::ALL[i];
            // A section without numbers for its choices, or with numbers from before
            // HEAD or a branch moved, reads them when it shows.
            s.forget_moved(history);
            reload |= s.needs_read(s.section)
                && !(s.section == Section::Pulls && s.pulls.attempted);
        }
        reload |= match s.section {
            Section::Activity => {
                let (read, asked) = activity_section(ui, &mut s, widgets);
                if asked {
                    export = Some(Section::Activity);
                }
                read
            }
            Section::Pulls => {
                let out = pulls::show(ui, &mut s.pulls, widgets, backend.as_ref());
                hosting = out.hosting;
                if out.export {
                    export = Some(Section::Pulls);
                }
                out.reload
            },
            Section::Treemap => treemap_section(ui, &mut s, widgets, downloads.as_ref()),
        };
        // The folder is looked up only when an export starts: it can mean disk reads.
        let folder = || crate::folder_browser::save_folder(scope.as_deref());
        match export {
            Some(Section::Pulls) => s.export = Some(export::State::new(&s.pulls, folder())),
            Some(Section::Activity) => {
                s.export = s.activity.clone().map(|activity| {
                    export::State::activity(
                        export::ActivitySnapshot {
                            activity,
                            repository,
                            scope: s.activity_for.clone().unwrap_or((s.range, ActivityScope::default())),
                            read_at: s.activity_at,
                        },
                        folder(),
                    )
                })
            }
            _ => {}
        }
        let browse = export::dialog(ui.ctx(), &mut s.export, widgets, backend.as_ref(), scope);
        let leave = close || hosting;
        if leave {
            s.park();
        }
        if let Some(owner) = owner {
            self.insights.insert(owner, s);
        }
        if leave {
            self.dialog = None;
            if hosting {
                self.dispatch(crate::app::Intent::Dialog(DialogKind::Hosting));
            }
            return;
        }
        if reload {
            self.start_statistics_read();
        }
        if let Some(folder) = browse {
            self.browse(crate::folder_browser::PickTarget::InsightsExportFolder, &folder);
        }
    }

    pub fn insights_export_open(&self) -> bool {
        self.statistics().is_some_and(|s| s.export.is_some())
    }
    pub(crate) fn insights_export_folder_picked(&mut self, path: &std::path::Path) {
        if let Some(e) = self
            .statistics_mut()
            .and_then(|s| s.export.as_mut())
            .filter(|e| !e.running())
        {
            e.dir = path.display().to_string();
        }
    }
}

/// A row of period choices; returns the one clicked, if it changed.
fn periods(
    ui: &mut Ui,
    current: Period,
    prefix: &str,
    widgets: &mut HashMap<String, UiControl>,
) -> Option<Period> {
    let items: Vec<(&str, bool)> = Period::ALL
        .iter()
        .map(|p| (p.label(), *p == current))
        .collect();
    let mut chosen = None;
    for (r, p) in pages::segments(ui, &items).iter().zip(Period::ALL) {
        pages::control(widgets, format!("{prefix}-{}", p.key()), r, ui.clip_rect());
        if r.clicked() && p != current {
            chosen = Some(p);
        }
    }
    chosen
}

/// The right end of a filter row: Export when `export` says whether it can be used,
/// "Show as table" or "Show charts", and when the numbers shown were read where the
/// row leaves room for it. Returns whether Export was clicked.
fn row_end(
    ui: &mut Ui,
    s: &mut State,
    widgets: &mut HashMap<String, UiControl>,
    read_at: Option<u64>,
    export: Option<bool>,
) -> bool {
    let mut clicked = false;
    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
        if let Some(enabled) = export {
            let r = ui.add_enabled(enabled, Button::new(crate::i18n::tr("Export")));
            pages::control(widgets, "stats-export", &r, ui.clip_rect());
            clicked = r.clicked();
        }
        let r = pages::ghost(ui, true, if s.table { crate::i18n::tr("Show charts") } else { crate::i18n::tr("Show as table") });
        pages::control(widgets, "stats-table", &r, ui.clip_rect());
        if r.clicked() {
            s.table = !s.table;
        }
        if let Some(read_at) = read_at.filter(|t| *t > 0) {
            let (when, at) = refreshed(read_at, ui.ctx());
            let text = crate::i18n::trf!("Refreshed {when}", when);
            let width = ui.painter().layout_no_wrap(text.clone(), FontId::proportional(12.0), FAINT()).size().x;
            if width + 8.0 <= ui.available_width() {
                let r = ui.label(RichText::new(text).size(12.0).color(FAINT()));
                pages::control(widgets, "stats-refreshed", &r, ui.clip_rect());
                r.on_hover_text(at);
            }
        }
    });
    clicked
}

/// A count's step on the ramp: 0 for none, then 1–4 in quarters of `max`.
fn step(n: usize, max: usize) -> usize {
    if n == 0 || max == 0 {
        0
    } else {
        n.saturating_mul(4).div_ceil(max).clamp(1, 4)
    }
}

/// Text on a ramp colour: dark on the two brightest steps.
fn ink_on(step: usize) -> Color32 {
    if step >= 3 { ACCENT_INK() } else { TEXT() }
}

const MONTHS: [&str; 12] = [
    crate::i18n::mark("Jan"), crate::i18n::mark("Feb"), crate::i18n::mark("Mar"), crate::i18n::mark("Apr"), crate::i18n::mark("May"), crate::i18n::mark("Jun"), crate::i18n::mark("Jul"), crate::i18n::mark("Aug"), crate::i18n::mark("Sep"), crate::i18n::mark("Oct"), crate::i18n::mark("Nov"), crate::i18n::mark("Dec"),
];

/// "Aug 24" for a day number (days since 1970-01-01).
fn month_day(days: i64) -> String {
    let when = date(days * 86_400, 0);
    let month: usize = when[5..7].parse().unwrap_or(1);
    let day: u32 = when[8..10].parse().unwrap_or(1);
    format!("{} {day}", crate::i18n::tr(MONTHS[month.clamp(1, 12) - 1]))
}

/// The year of a day number.
fn year_of(days: i64) -> String {
    date(days * 86_400, 0)[..4].to_owned()
}

/// A card with a title, an optional note beside it, and its content.
fn card(ui: &mut Ui, title: &str, note: &str, content: impl FnOnce(&mut Ui)) {
    Frame::new()
        .fill(PANEL())
        .stroke(Stroke::new(1.0, HAIRLINE()))
        .corner_radius(10)
        .inner_margin(Margin::symmetric(16, 14))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new(title).font(semibold(13.5)));
                if !note.is_empty() {
                    ui.add(Label::new(RichText::new(note).size(12.0).color(HINT())).truncate());
                }
            });
            ui.add_space(8.0);
            content(ui);
        });
}

/// Range and branch choices, tiles, charts and the heatmap. Returns whether to read
/// again and whether Export was clicked.
fn activity_section(ui: &mut Ui, s: &mut State, widgets: &mut HashMap<String, UiControl>) -> (bool, bool) {
    let (mut reload, mut export) = (false, false);
    pages::toolbar(ui, |ui| {
        if let Some(p) = periods(ui, s.range, "stats-range", widgets) {
            s.range = p;
            reload = true;
        }
        ui.add_space(6.0);
        let scopes = [(crate::i18n::tr("This branch"), !s.all_branches), (crate::i18n::tr("All branches"), s.all_branches)];
        for (i, r) in pages::segments(ui, &scopes).iter().enumerate() {
            pages::control(widgets, ["stats-scope-head", "stats-scope-all"][i], r, ui.clip_rect());
            if r.clicked() && (i == 1) != s.all_branches {
                s.all_branches = i == 1;
                reload = true;
            }
        }
        let read_at = s.activity.as_ref().map(|_| s.activity_at);
        let ready = s.activity.as_ref().is_some_and(|a| a.commits > 0) && !s.running();
        export = row_end(ui, s, widgets, read_at, Some(ready));
    });
    // The numbers shown stay, said for the choices they were read for, until the new
    // ones replace them.
    let Some(a) = s.activity.clone() else {
        Frame::new()
            .inner_margin(Margin::symmetric(24, 14))
            .show(ui, |ui| match &s.activity_error {
                Some(e) => pages::callout(ui, AMBER(), e),
                None => pages::loading(ui, crate::i18n::tr("Reading commit history…")),
            });
        return (reload, false);
    };
    let scroll = ScrollArea::vertical()
        .id_salt("stats-activity-scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            Frame::new()
                .inner_margin(Margin {
                    left: 24,
                    right: 24,
                    top: 4,
                    bottom: 20,
                })
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.spacing_mut().item_spacing.y = 14.0;
                    // A new read keeps the last result on screen; its one spinner is
                    // here, where the data appears.
                    if s.activity_running() {
                        pages::busy_line(ui, crate::i18n::tr("Reading commit history…"));
                    }
                    if let Some(e) = &s.activity_error {
                        pages::callout(ui, AMBER(), e);
                    }
                    tiles(ui, &a, s);
                    if a.limited {
                        pages::callout(
                            ui,
                            AMBER(),
                            &crate::i18n::trf!("Only the newest {count} commits are counted.", count = ACTIVITY_COMMITS),
                        );
                    }
                    if a.commits == 0 {
                        pages::empty(ui, crate::i18n::tr("No commits in this range."));
                        return;
                    }
                    let series = series(&a);
                    if ui.available_width() >= 900.0 {
                        ui.columns(2, |c| {
                            timeline(&mut c[0], &series, s.table);
                            authors(&mut c[1], &a, s.table);
                        });
                    } else {
                        timeline(ui, &series, s.table);
                        authors(ui, &a, s.table);
                    }
                    heatmap(ui, &a, s.table);
                });
        });
    pages::region(
        widgets,
        "stats-scroll",
        scroll.inner_rect.intersect(ui.clip_rect()),
        true,
    );
    (reload, export)
}

/// Four figures across: commits, authors, merges and the active span, said for the
/// choices they were read for.
fn tiles(ui: &mut Ui, a: &CommitActivity, s: &State) {
    let (range, all_branches) = s
        .activity_for
        .as_ref()
        .map_or((s.range, s.all_branches), |(range, scope)| (*range, scope.all_branches));
    let scope = if all_branches { crate::i18n::tr("on all branches") } else { crate::i18n::tr("on this branch") };
    let range = range.words();
    let (active, span) = match (a.first, a.last) {
        (Some(first), Some(last)) => {
            let (from, to) = (first.div_euclid(86_400), last.div_euclid(86_400));
            let text = if year_of(from) == year_of(to) {
                format!("{} – {}", month_day(from), month_day(to))
            } else {
                format!("{}, {} – {}, {}", month_day(from), year_of(from), month_day(to), year_of(to))
            };
            (text, crate::i18n::trf!("Days: {total}, with commits: {active}", total = to - from + 1, active = a.days))
        }
        _ => ("—".into(), crate::i18n::tr("no commits").into()),
    };
    let figures = [
        (crate::i18n::tr("Commits"), a.commits.to_string(), format!("{scope}, {range}"), None),
        (crate::i18n::tr("Authors"), a.authors.len().to_string(), crate::i18n::tr("grouped by email").into(), None),
        (crate::i18n::tr("Merges"), a.merges.to_string(), crate::i18n::tr("included in commits").into(), None),
        (crate::i18n::tr("Active"), active, span, None),
    ];
    figure_tiles(ui, &figures, 4);
}

/// A figure's change since the period before, and its tooltip.
type Change = Option<(String, String)>;

/// A figure's change ("+9"), as a small chip after its number.
fn change_chip(ui: &mut Ui, text: &str) -> Response {
    let galley = ui.painter().layout_no_wrap(text.to_owned(), semibold(11.0), TEXT_SOFT());
    let (rect, response) =
        ui.allocate_exact_size(vec2(galley.size().x + 12.0, 18.0), Sense::hover());
    ui.painter().rect_filled(rect, 9.0, CHIP());
    ui.painter().galley(
        pos2(rect.left() + 6.0, rect.center().y - galley.size().y / 2.0),
        galley,
        TEXT_SOFT(),
    );
    response
}

/// Figures as tiles sharing the width, `across` to a row: what each counts, the
/// number with how it changed when known, and a line about it.
fn figure_tiles(ui: &mut Ui, figures: &[(&str, String, String, Change)], across: usize) {
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.y = 8.0;
        for row in figures.chunks(across.max(1)) {
            ui.columns(row.len(), |columns| {
                for (ui, (label, value, note, change)) in columns.iter_mut().zip(row) {
                    Frame::new()
                        .fill(PANEL())
                        .stroke(Stroke::new(1.0, HAIRLINE()))
                        .corner_radius(10)
                        .inner_margin(Margin::symmetric(14, 12))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.spacing_mut().item_spacing.y = 3.0;
                            ui.add(Label::new(RichText::new(*label).size(12.0).color(HINT())).truncate());
                            // The change follows the number where both fit the tile;
                            // otherwise the number says it in its tooltip.
                            let fits = change.as_ref().is_some_and(|(text, _)| {
                                let width = |text: &str, font| ui.painter().layout_no_wrap(text.to_owned(), font, TEXT()).size().x;
                                width(value, semibold(21.0)) + 8.0 + width(text, semibold(11.0)) + 12.0
                                    <= ui.available_width()
                            });
                            match change {
                                Some((text, before)) if fits => {
                                    // As tall as the number alone, so tiles in a row
                                    // keep one height.
                                    ui.spacing_mut().interact_size.y = 0.0;
                                    ui.horizontal(|ui| {
                                        ui.spacing_mut().item_spacing.x = 8.0;
                                        ui.label(RichText::new(value).font(semibold(21.0)));
                                        change_chip(ui, text).on_hover_text(before);
                                    });
                                }
                                Some((text, before)) => {
                                    ui.add(Label::new(RichText::new(value).font(semibold(21.0))).truncate())
                                        .on_hover_text(format!("{text} · {before}"));
                                }
                                None => {
                                    ui.add(Label::new(RichText::new(value).font(semibold(21.0))).truncate());
                                }
                            }
                            ui.add(Label::new(RichText::new(note).size(12.0).color(FAINT())).truncate());
                        });
                }
            });
        }
    });
}

/// Commits over time: per week up to a year, then per month, then per year.
struct Series {
    title: &'static str,
    /// What one bucket is called in the table's heading.
    unit: &'static str,
    /// Axis label, full label and commits per bucket, oldest first.
    buckets: Vec<(String, String, usize)>,
}

fn series(a: &CommitActivity) -> Series {
    let counts: HashMap<i64, usize> = a.weeks.iter().copied().collect();
    if let (Some(&(first, _)), Some(&(last, _))) = (a.weeks.first(), a.weeks.last())
        && (last - first) / 7 < 53
    {
        let buckets = (0..=(last - first) / 7)
            .map(|i| {
                let monday = first + i * 7;
                let label = month_day(monday);
                (label.clone(), crate::i18n::trf!("Week of {date}", date = label), counts.get(&monday).copied().unwrap_or(0))
            })
            .collect();
        return Series { title: crate::i18n::tr("Commits per week"), unit: crate::i18n::tr("Week of"), buckets };
    }
    let (Some(&(y0, m0, _)), Some(&(y1, m1, _))) = (a.months.first(), a.months.last()) else {
        return Series { title: crate::i18n::tr("Commits per week"), unit: crate::i18n::tr("Week of"), buckets: Vec::new() };
    };
    let counts: HashMap<(i32, u32), usize> = a.months.iter().map(|&(y, m, n)| ((y, m), n)).collect();
    let span = (y1 - y0) as usize * 12 + m1 as usize + 1 - m0 as usize;
    if span <= 48 {
        let buckets = (0..span)
            .map(|i| {
                let month = m0 as usize - 1 + i;
                let (y, m) = (y0 + (month / 12) as i32, (month % 12) as u32 + 1);
                let label = format!("{} {y}", crate::i18n::tr(MONTHS[m as usize - 1]));
                (label.clone(), label, counts.get(&(y, m)).copied().unwrap_or(0))
            })
            .collect();
        return Series { title: crate::i18n::tr("Commits per month"), unit: crate::i18n::tr("Month"), buckets };
    }
    let buckets = (y0..=y1)
        .map(|y| {
            let n = (1..=12).map(|m| counts.get(&(y, m)).copied().unwrap_or(0)).sum();
            (y.to_string(), y.to_string(), n)
        })
        .collect();
    Series { title: crate::i18n::tr("Commits per year"), unit: crate::i18n::tr("Year"), buckets }
}

/// A step for gridlines: 1, 2 or 5 times a power of ten, for about three lines.
fn grid_step(max: usize) -> usize {
    let rough = (max.max(1) as f64 / 3.0).max(1.0);
    let power = 10f64.powi(rough.log10().floor() as i32);
    let step = [1.0, 2.0, 5.0, 10.0]
        .into_iter()
        .map(|k| k * power)
        .find(|s| *s >= rough)
        .unwrap_or(10.0 * power);
    step.max(1.0) as usize
}

/// Commits over time as bars with gridlines, or as a table.
fn timeline(ui: &mut Ui, series: &Series, table: bool) {
    card(ui, series.title, "", |ui| {
        if table {
            // The heading names the bucket ("Week of"), so rows show just its start.
            two_columns(ui, "stats-weeks-table", (series.unit, crate::i18n::tr("Commits")), series.buckets.iter().map(|(axis, _, n)| (axis.clone(), n.to_string())));
            return;
        }
        let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 190.0), Sense::hover());
        let painter = ui.painter_at(rect);
        let max = series.buckets.iter().map(|b| b.2).max().unwrap_or(0).max(1);
        let grid = grid_step(max);
        let top = max.div_ceil(grid) * grid;
        let plot = Rect::from_min_max(pos2(rect.left() + 30.0, rect.top() + 6.0), pos2(rect.right() - 4.0, rect.bottom() - 22.0));
        // Hairline gridlines with their values.
        let mut value = 0;
        while value <= top {
            let y = plot.bottom() - value as f32 / top as f32 * plot.height();
            painter.hline(plot.x_range(), y, Stroke::new(1.0, if value == 0 { BORDER() } else { HAIRLINE() }));
            painter.text(pos2(plot.left() - 8.0, y), Align2::RIGHT_CENTER, value.to_string(), FontId::proportional(11.0), FAINT());
            value += grid;
        }
        let n = series.buckets.len().max(1);
        let slot = plot.width() / n as f32;
        let width = (slot * 0.7).clamp(2.0, 24.0);
        // Labels about 70 points apart, so they never overlap.
        let every = (70.0 / slot).ceil().max(1.0) as usize;
        let pointer = response.hover_pos();
        let mut hovered = None;
        for (i, (axis, _, count)) in series.buckets.iter().enumerate() {
            let x = plot.left() + slot * (i as f32 + 0.5);
            let column = Rect::from_center_size(pos2(x, plot.center().y), vec2(slot, plot.height()));
            let hot = pointer.is_some_and(|p| column.contains(p));
            if hot {
                hovered = Some(i);
                painter.rect_filled(column, 2.0, HOVER());
            }
            if *count > 0 {
                let height = (*count as f32 / top as f32 * plot.height()).max(2.0);
                let bar = Rect::from_min_max(pos2(x - width / 2.0, plot.bottom() - height), pos2(x + width / 2.0, plot.bottom()));
                painter.rect_filled(bar, CornerRadius { nw: 4, ne: 4, sw: 0, se: 0 }, if hot { ACCENT_LIGHT() } else { BLUE() });
            }
            if i % every == 0 {
                painter.text(pos2(x, rect.bottom() - 10.0), Align2::CENTER_CENTER, axis, FontId::proportional(11.0), FAINT());
            }
        }
        if let Some(i) = hovered {
            let (_, label, count) = &series.buckets[i];
            response.on_hover_text_at_pointer(format!("{label} · {}", crate::i18n::trn!(*count, "{n} commit", "{n} commits")));
        }
    });
}

/// Who commits: one bar per author, the rest combined, or a table.
fn authors(ui: &mut Ui, a: &CommitActivity, table: bool) {
    const SHOWN: usize = 8;
    let total = a.commits.max(1);
    let mut rows: Vec<(String, usize)> = a.authors.iter().take(SHOWN).map(|x| (display(&x.name), x.commits)).collect();
    let others: usize = a.authors.iter().skip(SHOWN).map(|x| x.commits).sum();
    if others > 0 {
        rows.push((crate::i18n::trf!("Other authors: {count}", count = a.authors.len() - SHOWN), others));
    }
    let share = |n: usize| (n as f32 / total as f32 * 100.0).round() as usize;
    card(ui, crate::i18n::tr("Authors"), "", |ui| {
        if table {
            two_columns(ui, "stats-authors-table", (crate::i18n::tr("Author"), crate::i18n::tr("Commits")), rows.iter().map(|(name, n)| (name.clone(), format!("{n} · {}%", share(*n)))));
            return;
        }
        let max = rows.iter().map(|r| r.1).max().unwrap_or(1).max(1);
        ui.spacing_mut().item_spacing.y = 4.0;
        for (name, n) in &rows {
            let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::hover());
            let painter = ui.painter_at(rect);
            if response.hovered() {
                painter.rect_filled(rect, 4.0, HOVER());
            }
            let names = 140.0_f32.min(rect.width() * 0.4);
            let mut job = text::LayoutJob::simple_singleline(name.clone(), FontId::proportional(12.5), TEXT());
            job.wrap = text::TextWrapping::truncate_at_width(names - 8.0);
            let galley = painter.layout_job(job);
            painter.galley(pos2(rect.left() + 4.0, rect.center().y - galley.size().y / 2.0), galley, TEXT());
            let value = format!("{n} · {}%", share(*n));
            let value_width = 70.0;
            painter.text(pos2(rect.right() - 4.0, rect.center().y), Align2::RIGHT_CENTER, &value, FontId::proportional(12.0), MUTED());
            let track = Rect::from_min_max(pos2(rect.left() + names, rect.center().y - 4.0), pos2(rect.right() - value_width, rect.center().y + 4.0));
            let bar = Rect::from_min_size(track.min, vec2((track.width() * *n as f32 / max as f32).max(2.0), track.height()));
            painter.rect_filled(bar, CornerRadius { nw: 0, ne: 4, sw: 0, se: 4 }, BLUE());
            response.on_hover_text_at_pointer(crate::i18n::trf!("{author}: {commits} ({share}%)", author = name, commits = crate::i18n::trn!(*n, "{n} commit", "{n} commits"), share = share(*n)));
        }
    });
}

/// When commits happen: weekday by hour, coloured by the ramp, or a table.
fn heatmap(ui: &mut Ui, a: &CommitActivity, table: bool) {
    const DAYS: [&str; 7] = [crate::i18n::mark("Mon"), crate::i18n::mark("Tue"), crate::i18n::mark("Wed"), crate::i18n::mark("Thu"), crate::i18n::mark("Fri"), crate::i18n::mark("Sat"), crate::i18n::mark("Sun")];
    card(ui, crate::i18n::tr("When commits happen"), crate::i18n::tr("by weekday and hour, in each author's time zone"), |ui| {
        if table {
            let rows = DAYS.iter().enumerate().map(|(d, day)| {
                let busiest = (0..24).max_by_key(|&h| (a.heat[d][h], std::cmp::Reverse(h))).unwrap_or(0);
                let note = if a.weekdays[d] == 0 { String::new() } else { crate::i18n::trf!(" · most at {hour}", hour = format!("{busiest:02}:00")) };
                (crate::i18n::tr(day).to_owned(), format!("{}{note}", a.weekdays[d]))
            });
            two_columns(ui, "stats-heat-table", (crate::i18n::tr("Weekday"), crate::i18n::tr("Commits")), rows);
            return;
        }
        let max = a.heat.iter().flatten().copied().max().unwrap_or(0);
        let left = 40.0;
        // Hours share the width; rows keep one height so the grid stays compact.
        let cell = ((ui.available_width() - left) / 24.0).clamp(10.0, 64.0);
        let height = 20.0;
        let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 18.0 + 7.0 * height + 24.0), Sense::hover());
        let painter = ui.painter_at(rect);
        let grid = Rect::from_min_size(pos2(rect.left() + left, rect.top() + 18.0), vec2(cell * 24.0, height * 7.0));
        for h in (0..24).step_by(6) {
            painter.text(pos2(grid.left() + cell * h as f32 + 1.0, rect.top() + 7.0), Align2::LEFT_CENTER, format!("{h:02}"), FontId::proportional(11.0), FAINT());
        }
        let pointer = response.hover_pos();
        let mut hovered = None;
        for (d, day) in DAYS.iter().enumerate() {
            let y = grid.top() + height * d as f32;
            painter.text(pos2(rect.left(), y + height / 2.0), Align2::LEFT_CENTER, crate::i18n::tr(day), FontId::proportional(11.5), HINT());
            for h in 0..24 {
                let cell_rect = Rect::from_min_size(pos2(grid.left() + cell * h as f32, y), vec2(cell, height)).shrink(1.0);
                let n = a.heat[d][h];
                painter.rect_filled(cell_rect, 2.0, RAMP()[step(n, max)]);
                if pointer.is_some_and(|p| cell_rect.expand(1.0).contains(p)) {
                    hovered = Some((d, h, n));
                    painter.rect_stroke(cell_rect, 2.0, Stroke::new(1.5, TEXT()), StrokeKind::Outside);
                }
            }
        }
        // Legend: none to most.
        let y = rect.bottom() - 9.0;
        let mut x = grid.right() - 5.0 * 16.0 - 34.0;
        painter.text(pos2(x - 6.0, y), Align2::RIGHT_CENTER, crate::i18n::tr("None"), FontId::proportional(11.0), FAINT());
        for color in RAMP() {
            painter.rect_filled(Rect::from_min_size(pos2(x, y - 5.0), vec2(14.0, 10.0)), 2.0, color);
            x += 16.0;
        }
        painter.text(pos2(x + 4.0, y), Align2::LEFT_CENTER, crate::i18n::tr("Most"), FontId::proportional(11.0), FAINT());
        if let Some((d, h, n)) = hovered {
            response.on_hover_text_at_pointer(format!("{} {h:02}:00 · {}", crate::i18n::tr(DAYS[d]), crate::i18n::trn!(n, "{n} commit", "{n} commits")));
        }
    });
}

/// A plain two-column table with a heading row, scrolling past a dozen rows.
fn two_columns(
    ui: &mut Ui,
    id: &str,
    (left, right): (&str, &str),
    rows: impl Iterator<Item = (String, String)>,
) {
    let rows: Vec<_> = rows.collect();
    ui.horizontal(|ui| {
        ui.label(RichText::new(left).size(12.0).color(HINT()).strong());
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(RichText::new(right).size(12.0).color(HINT()).strong());
        });
    });
    pages::divider(ui);
    ScrollArea::vertical()
        .id_salt(id)
        .max_height(260.0)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            for (name, value) in &rows {
                ui.horizontal(|ui| {
                    ui.add(Label::new(RichText::new(name).size(12.5)).truncate());
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(RichText::new(value).size(12.5).color(MUTED()));
                    });
                });
            }
        });
}

/// Bytes as B, KB, MB or GB (powers of 1,024).
fn size(bytes: u64) -> String {
    let mut value = bytes as f64;
    for unit in ["B", "KB", "MB", "GB"] {
        if value < 1024.0 || unit == "GB" {
            return if unit == "B" {
                format!("{bytes} B")
            } else {
                format!("{value:.1} {unit}")
            };
        }
        value /= 1024.0;
    }
    unreachable!()
}

/// Rectangles for `sizes` (largest first) that fill `area` in proportion, keeping each
/// close to square (the squarified treemap of Bruls, Huizing and van Wijk). Entries
/// without size get an empty rectangle.
pub(crate) fn squarify(sizes: &[f64], area: Rect) -> Vec<Rect> {
    let mut rects = vec![Rect::NOTHING; sizes.len()];
    let total: f64 = sizes.iter().filter(|s| **s > 0.0).sum();
    if total <= 0.0 || area.width() <= 0.0 || area.height() <= 0.0 {
        return rects;
    }
    let scale = f64::from(area.width()) * f64::from(area.height()) / total;
    let items: Vec<(usize, f64)> = sizes
        .iter()
        .enumerate()
        .filter(|(_, s)| **s > 0.0)
        .map(|(i, s)| (i, s * scale))
        .collect();
    let (mut x, mut y) = (f64::from(area.left()), f64::from(area.top()));
    let (mut w, mut h) = (f64::from(area.width()), f64::from(area.height()));
    let worst = |row: &[(usize, f64)], side: f64| {
        let sum: f64 = row.iter().map(|r| r.1).sum();
        row.iter()
            .map(|r| (side * side * r.1 / (sum * sum)).max(sum * sum / (side * side * r.1)))
            .fold(0.0, f64::max)
    };
    let mut i = 0;
    while i < items.len() {
        let side = w.min(h);
        let mut j = i + 1;
        while j < items.len() && worst(&items[i..=j], side) <= worst(&items[i..j], side) {
            j += 1;
        }
        let row = &items[i..j];
        let sum: f64 = row.iter().map(|r| r.1).sum();
        let rect = |x: f64, y: f64, w: f64, h: f64| {
            Rect::from_min_size(pos2(x as f32, y as f32), vec2(w as f32, h as f32))
        };
        if w >= h {
            // A column at the left, as wide as the row's share of the height.
            let width = sum / h;
            let mut top = y;
            for &(k, s) in row {
                rects[k] = rect(x, top, width, s / width);
                top += s / width;
            }
            x += width;
            w -= width;
        } else {
            let height = sum / w;
            let mut left = x;
            for &(k, s) in row {
                rects[k] = rect(left, y, s / height, height);
                left += s / height;
            }
            y += height;
            h -= height;
        }
        i = j;
    }
    rects
}

/// One entry of the shown folder: a file or folder, or the smaller entries combined.
struct Cell<'a> {
    node: Option<&'a TreemapNode>,
    name: String,
    size: u64,
    changes: usize,
    files: usize,
}

/// The shown folder's entries: the largest, then the rest combined.
fn cells(folder: &TreemapNode) -> Vec<Cell<'_>> {
    let mut cells: Vec<Cell> = folder
        .children
        .iter()
        .take(CELLS)
        .map(|c| Cell {
            node: Some(c),
            name: display(&c.name),
            size: c.size,
            changes: c.changes,
            files: c.files,
        })
        .collect();
    let rest = &folder.children[folder.children.len().min(CELLS)..];
    if !rest.is_empty() {
        cells.push(Cell {
            node: None,
            name: crate::i18n::trf!("Smaller entries: {count}", count = rest.len()),
            size: rest.iter().map(|c| c.size).sum(),
            changes: rest.iter().map(|c| c.changes).max().unwrap_or(0),
            files: rest.iter().map(|c| c.files).sum(),
        });
    }
    cells
}

/// The files below `node` that changed most, with paths from it: commits, then size.
pub(crate) fn most_changed(node: &TreemapNode, limit: usize) -> Vec<(String, usize, u64)> {
    fn walk(node: &TreemapNode, prefix: &str, out: &mut Vec<(String, usize, u64)>) {
        for child in &node.children {
            let name = display(&child.name);
            let path = if prefix.is_empty() { name } else { format!("{prefix}/{name}") };
            if child.folder {
                walk(child, &path, out);
            } else if child.changes > 0 {
                out.push((path, child.changes, child.size));
            }
        }
    }
    let mut files = Vec::new();
    walk(node, "", &mut files);
    files.sort_by(|a, b| b.1.cmp(&a.1).then(b.2.cmp(&a.2)).then_with(|| a.0.cmp(&b.0)));
    files.truncate(limit);
    files
}

/// Period choice, breadcrumb, the treemap of the shown folder one level deep and its
/// most changed files. Returns true to read again.
fn treemap_section(
    ui: &mut Ui,
    s: &mut State,
    widgets: &mut HashMap<String, UiControl>,
    // A preview's downloads, for a commit whose files it doesn't have yet.
    preview: Option<&crate::preview_tab::Downloads>,
) -> bool {
    let mut reload = false;
    let mut zoom_to = None;
    let crumbs: Vec<String> = std::iter::once(crate::i18n::tr("Repository").to_owned())
        .chain(s.zoom.iter().map(|n| display(n)))
        .collect();
    pages::toolbar(ui, |ui| {
        if let Some(p) = periods(ui, s.period, "treemap-period", widgets) {
            s.period = p;
            reload = true;
        }
        ui.add_space(6.0);
        ui.spacing_mut().item_spacing.x = 4.0;
        for (i, crumb) in crumbs.iter().enumerate() {
            if i > 0 {
                ui.label(RichText::new("›").color(FAINT()));
            }
            let key = format!("treemap-crumb-{i}");
            if i + 1 == crumbs.len() {
                // The folder shown: plain text, not a link.
                let r = ui.label(RichText::new(crumb).size(12.5));
                let rect = r.rect.intersect(ui.clip_rect());
                if rect.is_positive() {
                    widgets.insert(key, UiControl { rect, enabled: false });
                }
            } else {
                let r = ui.add(Button::new(RichText::new(crumb).size(12.5).color(BLUE())).frame(false));
                pages::control(widgets, key, &r, ui.clip_rect());
                if r.clicked() {
                    zoom_to = Some(i);
                }
            }
        }
        ui.label(RichText::new(crate::i18n::tr("· all files at HEAD")).size(12.0).color(FAINT()));
        let read_at = s.treemap.as_ref().map(|_| s.treemap_at);
        row_end(ui, s, widgets, read_at, None);
    });
    // A clone, so zooming can change `s` while the cells borrow the treemap.
    let map = s.treemap.clone();
    let folder = map.as_deref().and_then(|m| folder_at(m, &s.zoom));
    let (Some(map), Some(folder)) = (map.as_deref(), folder) else {
        let (mut stop, mut retry) = (false, false);
        Frame::new()
            .inner_margin(Margin::symmetric(24, 14))
            .show(ui, |ui| {
                if let Some(e) = &s.treemap_error {
                    match crate::preview_tab::not_downloaded(ui, preview, "treemap-download-retry", widgets) {
                        Some(clicked) => retry = clicked,
                        None => pages::callout(ui, AMBER(), e),
                    }
                } else if let Some(clicked) = crate::preview_tab::downloading(ui, preview, crate::i18n::tr("Downloading the files of this commit"), "treemap-download-stop", widgets) {
                    stop = clicked;
                } else {
                    pages::loading(ui, crate::i18n::tr("Reading files and history…"));
                }
            });
        if stop {
            s.cancel();
        }
        reload |= retry;
        if let Some(i) = zoom_to {
            s.zoom.truncate(i);
        }
        return reload;
    };
    // The most changed files are worked out once per folder shown.
    let key = (Arc::as_ptr(s.treemap.as_ref().unwrap()) as usize, s.zoom.clone());
    if s.top.as_ref().is_none_or(|(k, _)| *k != key) {
        s.top = Some((key, most_changed(folder, 10)));
    }
    let mut open = None;
    let scroll = ScrollArea::vertical()
        .id_salt("stats-files-scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            Frame::new()
                .inner_margin(Margin {
                    left: 24,
                    right: 24,
                    top: 4,
                    bottom: 20,
                })
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.spacing_mut().item_spacing.y = 14.0;
                    // A new read keeps the last treemap on screen, with its one spinner.
                    if s.treemap_running() {
                        pages::busy_line(ui, crate::i18n::tr("Reading files and history…"));
                    }
                    if let Some(e) = &s.treemap_error {
                        pages::callout(ui, AMBER(), e);
                    }
                    // Said for the period the treemap shown was read for: a new read
                    // keeps it on screen until it is replaced.
                    let period = s.treemap_for.unwrap_or(s.period).phrase();
                    card(
                        ui,
                        crate::i18n::tr("Where changes happen"),
                        &crate::i18n::trf!("Area is size, colour is commits that changed it {period} · click a folder to zoom in", period),
                        |ui| {
                            pages::note(
                                ui,
                                &crate::i18n::trf!(
                                    "{files} · {size} · HEAD {commit} · commits read: {count}{limit}",
                                    files = crate::i18n::trn!(folder.files, "{n} file", "{n} files"),
                                    size = size(folder.size),
                                    commit = &map.commit.as_str()[..8],
                                    count = map.commits,
                                    limit = if map.limited { crate::i18n::tr(", the newest only") } else { "" }
                                ),
                            );
                            if s.table {
                                entries_table(ui, folder);
                            } else {
                                open = treemap(ui, folder, s.zoom.len(), &s.zoom, widgets, period);
                            }
                        },
                    );
                    card(ui, crate::i18n::tr("Most changed files"), &crate::i18n::trf!("{period}, in this folder", period), |ui| {
                        let top = s.top.as_ref().map(|(_, files)| files.as_slice()).unwrap_or_default();
                        if top.is_empty() {
                            pages::note(ui, crate::i18n::tr("No file here changed in this period."));
                            return;
                        }
                        let rows = top.iter().map(|(path, changes, bytes)| {
                            [path.clone(), changes.to_string(), size(*bytes)]
                        });
                        three_columns(ui, "stats-top-files", [crate::i18n::tr("File"), crate::i18n::tr("Commits"), crate::i18n::tr("Size")], rows);
                    });
                });
        });
    pages::region(
        widgets,
        "stats-scroll",
        scroll.inner_rect.intersect(ui.clip_rect()),
        true,
    );
    if let Some(name) = open {
        s.zoom.push(name);
    }
    if let Some(i) = zoom_to {
        s.zoom.truncate(i);
    }
    reload
}

/// The shown folder's entries as a treemap, each folder with its own entries inside,
/// on one colour scale. Returns a folder to open.
fn treemap(
    ui: &mut Ui,
    folder: &TreemapNode,
    depth: usize,
    zoom: &[Vec<u8>],
    widgets: &mut HashMap<String, UiControl>,
    period: &str,
) -> Option<Vec<u8>> {
    let cells = cells(folder);
    let max = cells
        .iter()
        .map(|c| c.changes)
        .chain(
            cells
                .iter()
                .filter_map(|c| c.node)
                .flat_map(|n| n.children.iter().map(|k| k.changes)),
        )
        .max()
        .unwrap_or(0);
    let (area, _) = ui.allocate_exact_size(vec2(ui.available_width(), 340.0), Sense::hover());
    let sizes: Vec<f64> = cells.iter().map(|c| c.size as f64).collect();
    let rects = squarify(&sizes, area);
    let painter = ui.painter_at(area.expand(1.0));
    if cells.is_empty() || folder.size == 0 {
        painter.text(
            area.center(),
            Align2::CENTER_CENTER,
            crate::i18n::tr("No files with content here."),
            FontId::proportional(13.0),
            MUTED(),
        );
    }
    let mut open = None;
    for (i, (cell, rect)) in cells.iter().zip(&rects).enumerate() {
        if !rect.is_positive() {
            continue;
        }
        let level = step(cell.changes, max);
        let folder_cell = cell.node.is_some_and(|n| n.folder);
        painter.rect(*rect, 3.0, RAMP()[level], Stroke::new(1.5, BG()), StrokeKind::Inside);
        let ink = ink_on(level);
        // One level deeper: a folder's own entries, under its title.
        let inner = Rect::from_min_max(rect.min + vec2(4.0, 38.0), rect.max - vec2(4.0, 4.0));
        if let Some(node) = cell.node.filter(|n| n.folder)
            && inner.width() > 60.0
            && inner.height() > 30.0
        {
            let kids: Vec<&TreemapNode> = node.children.iter().take(80).collect();
            let kid_sizes: Vec<f64> = kids.iter().map(|k| k.size as f64).collect();
            for (kid, r) in kids.iter().zip(squarify(&kid_sizes, inner)) {
                if !r.is_positive() {
                    continue;
                }
                let kid_level = step(kid.changes, max);
                painter.rect(r, 2.0, RAMP()[kid_level], Stroke::new(1.0, BG()), StrokeKind::Inside);
                if r.width() > 46.0 && r.height() > 18.0 {
                    let name = if kid.folder {
                        format!("{}/", display(&kid.name))
                    } else {
                        display(&kid.name)
                    };
                    let mut job = text::LayoutJob::simple_singleline(name, FontId::proportional(11.0), ink_on(kid_level));
                    job.wrap = text::TextWrapping::truncate_at_width(r.width() - 8.0);
                    painter.galley(r.min + vec2(4.0, 3.0), painter.layout_job(job), ink_on(kid_level));
                }
            }
        }
        if rect.width() > 56.0 && rect.height() > 20.0 {
            let title = if folder_cell {
                format!("{}/", cell.name)
            } else {
                cell.name.clone()
            };
            let mut job = text::LayoutJob::simple_singleline(title, semibold(12.5), ink);
            job.wrap = text::TextWrapping::truncate_at_width(rect.width() - 12.0);
            painter.galley(rect.min + vec2(6.0, 5.0), painter.layout_job(job), ink);
            if rect.height() > 38.0 {
                let commits = crate::i18n::trn!(cell.changes, "{n} commit", "{n} commits");
                let mut job = text::LayoutJob::simple_singleline(
                    format!("{commits} · {}", size(cell.size)),
                    FontId::proportional(11.0),
                    ink.gamma_multiply(0.85),
                );
                job.wrap = text::TextWrapping::truncate_at_width(rect.width() - 12.0);
                painter.galley(rect.min + vec2(6.0, 21.0), painter.layout_job(job), ink);
            }
        }
        let r = ui.interact(
            *rect,
            Id::new(("treemap-cell", depth, i)),
            if folder_cell {
                Sense::click()
            } else {
                Sense::hover()
            },
        );
        pages::control(widgets, format!("treemap-cell-{i}"), &r, ui.clip_rect());
        let path: String = zoom
            .iter()
            .map(|n| display(n))
            .chain(std::iter::once(cell.name.clone()))
            .collect::<Vec<_>>()
            .join("/");
        let files = if cell.files == 1 {
            String::new()
        } else {
            format!(" · {}", crate::i18n::trn!(cell.files, "{n} file", "{n} files"))
        };
        let r = r.on_hover_text_at_pointer(crate::i18n::trf!(
            "{path}\n{size}{files} · {commits} {period}{hint}",
            path = format!("{path}{}", if folder_cell { "/" } else { "" }),
            size = size(cell.size), files,
            commits = crate::i18n::trn!(cell.changes, "{n} commit", "{n} commits"), period,
            hint = if folder_cell { crate::i18n::tr("\nClick to open this folder") } else { "" }
        ));
        if folder_cell && r.clicked() {
            open = cell.node.map(|n| n.name.clone());
        }
    }
    // Legend: no commits, the ramp, then the most here.
    let (legend, _) = ui.allocate_exact_size(vec2(ui.available_width(), 18.0), Sense::hover());
    let painter = ui.painter_at(legend);
    let y = legend.center().y;
    let mut x = legend.left();
    let galley = painter.layout_no_wrap(crate::i18n::trn!(0, "{n} commit", "{n} commits"), FontId::proportional(11.0), FAINT());
    let width = galley.size().x;
    painter.galley(pos2(x, y - galley.size().y / 2.0), galley, FAINT());
    x += width + 8.0;
    for color in RAMP() {
        painter.rect_filled(Rect::from_min_size(pos2(x, y - 5.0), vec2(22.0, 10.0)), 2.0, color);
        x += 24.0;
    }
    painter.text(
        pos2(x + 6.0, y),
        Align2::LEFT_CENTER,
        crate::i18n::trn!(max, "{n} commit", "{n} commits"),
        FontId::proportional(11.0),
        FAINT(),
    );
    open
}

/// The shown folder's entries as a table.
fn entries_table(ui: &mut Ui, folder: &TreemapNode) {
    let rows = folder.children.iter().map(|c| {
        let name = if c.folder {
            format!("{}/", display(&c.name))
        } else {
            display(&c.name)
        };
        [name, c.changes.to_string(), size(c.size)]
    });
    three_columns(ui, "stats-entries-table", [crate::i18n::tr("Name"), crate::i18n::tr("Commits"), crate::i18n::tr("Size")], rows);
}

/// One table row: the name, cut short, then two right-aligned values.
fn table_row(ui: &mut Ui, mut name: text::LayoutJob, values: [RichText; 2]) {
    const VALUE: f32 = 90.0;
    ui.horizontal(|ui| {
        // No gaps between the cells: with them each row would be wider than the
        // table, and a scrolling table would widen with every row.
        ui.spacing_mut().item_spacing.x = 0.0;
        let width = (ui.available_width() - VALUE * 2.0).max(60.0);
        name.wrap = text::TextWrapping::truncate_at_width(width);
        ui.allocate_ui_with_layout(vec2(width, 18.0), Layout::left_to_right(Align::Center), |ui| {
            ui.set_width(width);
            ui.label(name);
        });
        for value in values {
            ui.allocate_ui_with_layout(vec2(VALUE, 18.0), Layout::right_to_left(Align::Center), |ui| {
                ui.set_width(VALUE);
                ui.label(value);
            });
        }
    });
}

/// A table: names (a path's folder dimmed), then two right-aligned value columns.
fn three_columns(
    ui: &mut Ui,
    id: &str,
    heads: [&str; 3],
    rows: impl Iterator<Item = [String; 3]>,
) {
    let head = |text: &str| RichText::new(text).size(12.0).color(HINT()).strong();
    table_row(
        ui,
        text::LayoutJob::simple_singleline(heads[0].into(), semibold(12.0), HINT()),
        [head(heads[1]), head(heads[2])],
    );
    pages::divider(ui);
    ScrollArea::vertical()
        .id_salt(id)
        .max_height(300.0)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 3.0;
            for [name, a, b] in rows {
                let (folder, file) = name
                    .rsplit_once('/')
                    .filter(|(_, file)| !file.is_empty())
                    .unwrap_or(("", name.as_str()));
                let mut job = text::LayoutJob::default();
                if !folder.is_empty() {
                    job.append(&format!("{folder}/"), 0.0, TextFormat::simple(FontId::proportional(12.5), FAINT()));
                }
                job.append(file, 0.0, TextFormat::simple(FontId::proportional(12.5), TEXT()));
                table_row(
                    ui,
                    job,
                    [
                        RichText::new(a).size(12.5).color(MUTED()),
                        RichText::new(b).size(12.5).color(MUTED()),
                    ],
                );
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn squarified_cells_fill_the_area_in_proportion() {
        let area = Rect::from_min_size(pos2(10.0, 20.0), vec2(600.0, 400.0));
        let sizes = [6.0, 6.0, 4.0, 3.0, 2.0, 2.0, 1.0, 0.0];
        let rects = squarify(&sizes, area);
        let total: f64 = sizes.iter().sum();
        for (rect, s) in rects.iter().zip(sizes) {
            if s == 0.0 {
                assert!(!rect.is_positive());
                continue;
            }
            // Each area is its share, every cell lies inside, and none is a sliver.
            let share = f64::from(rect.area()) / f64::from(area.area());
            assert!((share - s / total).abs() < 1e-3, "{share} vs {}", s / total);
            assert!(area.expand(0.01).contains_rect(*rect));
            assert!(rect.aspect_ratio().max(1.0 / rect.aspect_ratio()) < 4.0);
        }
        // No two cells overlap.
        for (i, a) in rects.iter().enumerate() {
            for b in &rects[i + 1..] {
                assert!(a.intersect(*b).area() < 0.01 || !b.is_positive());
            }
        }
        assert!(squarify(&[0.0, 0.0], area).iter().all(|r| !r.is_positive()));
        assert!(squarify(&[], area).is_empty());
    }
    /// The page through real frames: activity, a narrower range, the treemap with a
    /// folder opened and the breadcrumb back.
    #[test]
    fn the_page_reads_activity_and_zooms_the_treemap() {
        use crate::{
            app::Intent,
            batch::flow_tests::{click, frame, open},
            test_common as common,
        };
        use std::time::{Duration, Instant};
        common::isolated(
            "ROCKET_STATISTICS_UI_CASE",
            "statistics::tests::the_page_reads_activity_and_zooms_the_treemap",
            || {
                let f = common::Fixture::new();
                f.write("src/a.rs", "a".repeat(300));
                f.write("src/b.rs", "b".repeat(100));
                f.write("README", "readme");
                f.commit("one");
                f.write("src/a.rs", "A".repeat(300));
                f.commit("two");
                let ctx = Context::default();
                let mut app = open(&ctx, &f);
                // Statistics reads are not `busy()`; wait for the page itself.
                let settle = |app: &mut Rocket| {
                    let deadline = Instant::now() + Duration::from_secs(60);
                    while app.statistics().is_some_and(State::running) {
                        app.poll(&ctx);
                        assert!(Instant::now() < deadline, "statistics did not finish");
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    frame(&ctx, app, vec![]);
                    frame(&ctx, app, vec![]);
                };
                fn s(app: &Rocket) -> &State {
                    app.statistics().unwrap()
                }
                app.dispatch(Intent::Dialog(DialogKind::Statistics));
                settle(&mut app);
                let a = s(&app).activity.clone().unwrap();
                assert_eq!((a.commits, a.authors.len(), a.merges), (2, 1, 0));
                // The fixture's commits are from 2026-01-01, outside the last 30 days.
                click(&ctx, &mut app, "stats-range-30");
                settle(&mut app);
                assert_eq!(s(&app).range, Period::Days30);
                assert_eq!(s(&app).activity.as_ref().unwrap().commits, 0);
                click(&ctx, &mut app, "stats-range-all");
                settle(&mut app);
                assert_eq!(s(&app).activity.as_ref().unwrap().commits, 2);
                // The treemap over all history: src first (largest), its two changes.
                click(&ctx, &mut app, "stats-section-treemap");
                settle(&mut app);
                click(&ctx, &mut app, "treemap-period-all");
                settle(&mut app);
                let folder = s(&app).shown().unwrap().clone();
                assert_eq!((folder.size, folder.files, folder.changes), (406, 3, 2));
                assert_eq!(folder.children[0].name, b"src");
                assert_eq!(
                    (folder.children[0].changes, folder.children[1].changes),
                    (2, 1)
                );
                click(&ctx, &mut app, "treemap-cell-0");
                assert_eq!(s(&app).zoom, [b"src".to_vec()]);
                assert!(app.frame_widgets.contains_key("treemap-crumb-0"));
                click(&ctx, &mut app, "treemap-crumb-0");
                assert!(s(&app).zoom.is_empty());
                click(&ctx, &mut app, "dialog-cancel");
                assert!(app.dialog.is_none());
            },
        );
    }
    /// Leaving the page keeps a tab's numbers and choices: coming back shows them without
    /// a read, and a read that leaving stopped is done again for the choice it was for.
    #[test]
    fn the_page_keeps_its_numbers_and_choices_while_the_tab_is_open() {
        use crate::{
            app::Intent,
            batch::flow_tests::{click, frame, open, wait},
            test_common as common,
        };
        use std::time::{Duration, Instant};
        common::isolated(
            "ROCKET_STATISTICS_UI_CASE",
            "statistics::tests::the_page_keeps_its_numbers_and_choices_while_the_tab_is_open",
            || {
                let f = common::Fixture::new();
                f.write("a.txt", "a");
                f.commit("one");
                let ctx = Context::default();
                let mut app = open(&ctx, &f);
                let settle = |app: &mut Rocket| {
                    let deadline = Instant::now() + Duration::from_secs(60);
                    while app.statistics().is_some_and(State::running) {
                        app.poll(&ctx);
                        assert!(Instant::now() < deadline, "statistics did not finish");
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    frame(&ctx, app, vec![]);
                    frame(&ctx, app, vec![]);
                };
                app.dispatch(Intent::Dialog(DialogKind::Statistics));
                settle(&mut app);
                click(&ctx, &mut app, "stats-scope-all");
                settle(&mut app);
                let s = app.statistics().unwrap();
                let (read, at) = (s.activity.clone().unwrap(), s.activity_at);
                assert!(s.all_branches && at > 0);
                // The row says when the numbers were read.
                assert!(app.frame_widgets.contains_key("stats-refreshed"));
                assert!(app.frame_widgets["stats-export"].enabled);
                click(&ctx, &mut app, "dialog-cancel");
                assert!(app.dialog.is_none());
                app.poll(&ctx);
                assert_eq!(app.insights.len(), 1);
                // Back: the same numbers at once, from the tab, with no read.
                app.dispatch(Intent::Dialog(DialogKind::Statistics));
                let s = app.statistics().unwrap();
                assert!(!s.running() && s.all_branches && s.activity_at == at);
                assert!(Arc::ptr_eq(s.activity.as_ref().unwrap(), &read));
                // A choice that needs a read, then another page before it ends: the read
                // stops, and the numbers kept are still said for the choice they were for.
                click(&ctx, &mut app, "stats-range-30");
                assert!(app.statistics().unwrap().activity_running());
                app.dispatch(Intent::Dialog(DialogKind::Tags));
                app.poll(&ctx);
                let parked = app.insights.values().next().unwrap();
                assert!(!parked.running());
                assert_eq!(parked.range, Period::Days30);
                assert_eq!(parked.activity_for.as_ref().map(|f| f.0), Some(Period::All));
                app.dispatch(Intent::Dialog(DialogKind::Statistics));
                assert!(app.statistics().unwrap().activity_running(), "the new choice is read");
                settle(&mut app);
                // The fixture's commit is from 2026-01-01, outside the last 30 days.
                assert_eq!(app.statistics().unwrap().activity.as_ref().unwrap().commits, 0);
                // HEAD moves while the page is closed: what it kept was read at the old
                // HEAD, so coming back reads again instead of showing it as this branch's.
                click(&ctx, &mut app, "dialog-cancel");
                f.write("b.txt", "b");
                f.commit("two");
                app.dispatch(Intent::Refresh);
                wait(&ctx, &mut app);
                app.dispatch(Intent::Dialog(DialogKind::Statistics));
                assert!(app.statistics().unwrap().activity_running(), "a moved HEAD is read again");
                settle(&mut app);
                // Nothing moved since: back again, no read.
                click(&ctx, &mut app, "dialog-cancel");
                app.dispatch(Intent::Dialog(DialogKind::Statistics));
                assert!(!app.statistics().unwrap().activity_running());
                // What a saved view keeps, and a page started from it.
                let view = View::capture(app.statistics().unwrap());
                assert_eq!((view.range, view.all_branches, view.section), (Period::Days30, true, Section::Activity));
                assert_eq!(View::capture(&State::from_view(&view)), view);
                // Closing the tab takes its page along.
                let id = app.tabs[0].id;
                app.dispatch(Intent::CloseTab(id));
                app.poll(&ctx);
                assert!(app.tabs.is_empty() && app.insights.is_empty());
            },
        );
    }
    #[test]
    fn numbers_read_before_head_or_a_branch_moved_are_dropped() {
        let mut s = State::default();
        let this_branch = ActivityScope { all_branches: false, since: None };
        s.activity = Some(Arc::new(CommitActivity::default()));
        (s.activity_for, s.activity_history) = (Some((Period::All, this_branch)), Some(1));
        (s.treemap_for, s.treemap_history) = (Some(Period::Year), Some(1));
        assert!(!s.needs_read(Section::Activity));
        // Before the first status read nothing can be told, so nothing is dropped.
        s.forget_moved(None);
        assert!(s.activity.is_some() && s.treemap_for.is_some());
        // Another branch moved: this branch's numbers and the files at HEAD stay.
        s.forget_moved(Some(History { head: 1, all: 9 }));
        assert!(s.activity.is_some() && s.treemap_for.is_some());
        // All branches count every tip, so the same move drops them.
        let all = ActivityScope { all_branches: true, since: None };
        s.activity_for = Some((Period::All, all));
        s.forget_moved(Some(History { head: 1, all: 9 }));
        assert!(s.activity.is_none() && s.activity_for.is_none());
        assert!(s.needs_read(Section::Activity));
        // HEAD moved: the treemap goes too.
        s.forget_moved(Some(History { head: 2, all: 9 }));
        assert!(s.treemap_for.is_none() && s.needs_read(Section::Treemap));
    }
    #[test]
    fn sizes_steps_and_buckets() {
        assert_eq!(size(512), "512 B");
        assert_eq!(size(1536), "1.5 KB");
        assert_eq!(size(5 * 1024 * 1024), "5.0 MB");
        // No commits is its own step; the most reaches the brightest.
        assert_eq!([0, 1, 3, 5, 6, 10].map(|n| step(n, 10)), [0, 1, 2, 2, 3, 4]);
        assert_eq!(step(3, 0), 0);
        assert_eq!([1, 3, 6, 7, 40, 250].map(grid_step), [1, 1, 2, 5, 20, 100]);
        // Weeks from the first to the last with commits, empty ones included.
        let a = CommitActivity {
            weeks: vec![(19_737, 2), (19_751, 1)],
            ..Default::default()
        };
        let weeks = series(&a);
        assert_eq!(weeks.title, "Commits per week");
        let counts: Vec<_> = weeks.buckets.iter().map(|b| (b.0.as_str(), b.2)).collect();
        assert_eq!(counts, [("Jan 15", 2), ("Jan 22", 0), ("Jan 29", 1)]);
        assert_eq!(month_day(19_737), "Jan 15");
    }
    #[test]
    fn most_changed_files_keep_their_paths_and_order() {
        let file = |name: &str, size: u64, changes: usize| TreemapNode {
            name: name.into(),
            folder: false,
            size,
            files: 1,
            changes,
            children: Vec::new(),
        };
        let root = TreemapNode {
            name: Vec::new(),
            folder: true,
            size: 0,
            files: 4,
            changes: 0,
            children: vec![
                TreemapNode {
                    name: b"src".to_vec(),
                    folder: true,
                    size: 0,
                    files: 2,
                    changes: 0,
                    children: vec![file("a.rs", 10, 3), file("b.rs", 99, 0)],
                },
                file("README", 50, 3),
                file("old", 5, 1),
            ],
        };
        assert_eq!(
            most_changed(&root, 2),
            [("README".to_owned(), 3, 50), ("src/a.rs".to_owned(), 3, 10)]
        );
    }
}
