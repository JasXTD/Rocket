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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Section {
    #[default]
    Activity,
    Treemap,
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
            Self::Days30 => "in the last 30 days",
            Self::Days90 => "in the last 90 days",
            Self::Year => "in the last year",
            Self::All => "in all history",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Days30 => "30 days",
            Self::Days90 => "90 days",
            Self::Year => "1 year",
            Self::All => "All",
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

const STOPPED: &str = "Stopped. Choose Refresh to read again.";
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
    activity_task: Option<ReadTask<CommitActivity>>,
    treemap_task: Option<ReadTask<ActivityTreemap>>,
    /// The shown folder's most changed files, for the treemap and zoom they were
    /// worked out for.
    top: Option<TopFiles>,
}
type TopFiles = ((usize, Vec<Vec<u8>>), Vec<(String, usize, u64)>);
impl Default for State {
    fn default() -> Self {
        Self {
            section: Section::Activity,
            range: Period::All,
            all_branches: false,
            period: Period::Year,
            activity: None,
            treemap: None,
            zoom: Vec::new(),
            activity_error: None,
            treemap_error: None,
            table: false,
            activity_task: None,
            treemap_task: None,
            top: None,
        }
    }
}
impl State {
    pub fn running(&self) -> bool {
        self.activity_task.is_some() || self.treemap_task.is_some()
    }
    pub fn activity_running(&self) -> bool {
        self.activity_task.is_some()
    }
    pub fn treemap_running(&self) -> bool {
        self.treemap_task.is_some()
    }
    pub fn cancel(&self) {
        if let Some(t) = &self.activity_task {
            t.cancel();
        }
        if let Some(t) = &self.treemap_task {
            t.cancel();
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
    /// Read the shown section of Repository Statistics, replacing an older read of it.
    pub(crate) fn start_statistics_read(&mut self) {
        let Some(d) = &mut self.dialog else { return };
        if !matches!(d.kind, DialogKind::Statistics) {
            return;
        }
        let s = d.statistics.get_or_insert_with(Default::default);
        let path = self
            .tabs
            .iter()
            .find(|t| Some(t.id) == d.owner)
            .map(|t| t.path.clone());
        let (Some(backend), Some(path)) = (self.backend.clone(), path) else {
            return;
        };
        match s.section {
            Section::Activity => {
                if let Some(t) = s.activity_task.take() {
                    t.cancel();
                }
                s.activity_error = None;
                let scope = ActivityScope {
                    all_branches: s.all_branches,
                    since: s.range.since(),
                };
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
        let Some(s) = self.dialog.as_mut().and_then(|d| d.statistics.as_mut()) else {
            return;
        };
        let message = |e: &Error| match e {
            Error::Cancelled => STOPPED.to_owned(),
            e => user_error(e),
        };
        if let Some(result) = s.activity_task.as_mut().and_then(ReadTask::try_finish) {
            s.activity_task = None;
            match result {
                Ok(a) => s.activity = Some(Arc::new(a)),
                Err(e) => s.activity_error = Some(message(&e)),
            }
        }
        if let Some(result) = s.treemap_task.as_mut().and_then(ReadTask::try_finish) {
            s.treemap_task = None;
            match result {
                Ok(map) => {
                    s.treemap = Some(Arc::new(map));
                    if s.shown().is_none() {
                        s.zoom.clear();
                    }
                }
                Err(e) => s.treemap_error = Some(message(&e)),
            }
        }
    }
    pub(crate) fn statistics_page(&mut self, ui: &mut Ui) {
        let downloads = self.tab().and_then(crate::preview_tab::downloads);
        let Some(mut d) = self.dialog.take() else {
            return;
        };
        let s = d.statistics.get_or_insert_with(Default::default);
        let widgets = &mut self.frame_widgets;
        let (mut reload, mut close) = (false, false);
        let running = match s.section {
            Section::Activity => s.activity_running(),
            Section::Treemap => s.treemap_running(),
        };
        pages::header(
            ui,
            Icon::Activity,
            "Statistics",
            "Who commits, when, and which files change most · read-only",
            |ui| {
                let r = pages::ghost(ui, true, "Close");
                pages::control(widgets, "dialog-cancel", &r, ui.clip_rect());
                close = r.clicked();
                let r = pages::ghost(ui, running, "Stop");
                pages::control(widgets, "stats-cancel", &r, ui.clip_rect());
                if r.clicked() {
                    s.cancel();
                }
                let r = crate::chrome::icon_button(ui, Icon::Refresh, !running, false, 32.0)
                    .on_hover_text("Read again");
                pages::control(widgets, "stats-refresh", &r, ui.clip_rect());
                reload |= r.clicked();
            },
        );
        let tabs = [
            pages::TabItem {
                label: "Activity",
                count: None,
                selected: s.section == Section::Activity,
                enabled: true,
                key: "stats-section-activity",
            },
            pages::TabItem {
                label: "Files",
                count: None,
                selected: s.section == Section::Treemap,
                enabled: true,
                key: "stats-section-treemap",
            },
        ];
        if let Some(i) = pages::tabs(ui, &tabs, widgets) {
            s.section = [Section::Activity, Section::Treemap][i];
            // The first visit of a section reads it.
            reload |= match s.section {
                Section::Activity => s.activity.is_none() && !s.activity_running(),
                Section::Treemap => s.treemap.is_none() && !s.treemap_running(),
            };
        }
        reload |= match s.section {
            Section::Activity => activity_section(ui, s, widgets),
            Section::Treemap => treemap_section(ui, s, widgets, downloads.as_ref()),
        };
        if close {
            s.cancel();
        } else {
            self.dialog = Some(d);
            if reload {
                self.start_statistics_read();
            }
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

/// "Show as table" or "Show charts", at the right of the filter row.
fn table_toggle(ui: &mut Ui, s: &mut State, widgets: &mut HashMap<String, UiControl>) {
    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
        let r = pages::ghost(ui, true, if s.table { "Show charts" } else { "Show as table" });
        pages::control(widgets, "stats-table", &r, ui.clip_rect());
        if r.clicked() {
            s.table = !s.table;
        }
    });
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
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// "Aug 24" for a day number (days since 1970-01-01).
fn month_day(days: i64) -> String {
    let when = date(days * 86_400, 0);
    let month: usize = when[5..7].parse().unwrap_or(1);
    let day: u32 = when[8..10].parse().unwrap_or(1);
    format!("{} {day}", MONTHS[month.clamp(1, 12) - 1])
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

/// Range and branch choices, tiles, charts and the heatmap. Returns true to read again.
fn activity_section(ui: &mut Ui, s: &mut State, widgets: &mut HashMap<String, UiControl>) -> bool {
    let mut reload = false;
    pages::toolbar(ui, |ui| {
        if let Some(p) = periods(ui, s.range, "stats-range", widgets) {
            s.range = p;
            reload = true;
        }
        ui.add_space(6.0);
        let scopes = [("This branch", !s.all_branches), ("All branches", s.all_branches)];
        for (i, r) in pages::segments(ui, &scopes).iter().enumerate() {
            pages::control(widgets, ["stats-scope-head", "stats-scope-all"][i], r, ui.clip_rect());
            if r.clicked() && (i == 1) != s.all_branches {
                s.all_branches = i == 1;
                reload = true;
            }
        }
        table_toggle(ui, s, widgets);
    });
    let Some(a) = s.activity.clone() else {
        Frame::new()
            .inner_margin(Margin::symmetric(24, 14))
            .show(ui, |ui| match &s.activity_error {
                Some(e) => pages::callout(ui, AMBER(), e),
                None => pages::loading(ui, "Reading commit history…"),
            });
        return reload;
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
                        pages::busy_line(ui, "Reading commit history…");
                    }
                    if let Some(e) = &s.activity_error {
                        pages::callout(ui, AMBER(), e);
                    }
                    tiles(ui, &a, s);
                    if a.limited {
                        pages::callout(
                            ui,
                            AMBER(),
                            &format!("Only the newest {ACTIVITY_COMMITS} commits are counted."),
                        );
                    }
                    if a.commits == 0 {
                        pages::empty(ui, "No commits in this range.");
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
    reload
}

/// Four figures across: commits, authors, merges and the active span.
fn tiles(ui: &mut Ui, a: &CommitActivity, s: &State) {
    let scope = if s.all_branches { "on all branches" } else { "on this branch" };
    let range = match s.range {
        Period::Days30 => "last 30 days",
        Period::Days90 => "last 90 days",
        Period::Year => "last year",
        Period::All => "all history",
    };
    let (active, span) = match (a.first, a.last) {
        (Some(first), Some(last)) => {
            let (from, to) = (first.div_euclid(86_400), last.div_euclid(86_400));
            let text = if year_of(from) == year_of(to) {
                format!("{} – {}", month_day(from), month_day(to))
            } else {
                format!("{}, {} – {}, {}", month_day(from), year_of(from), month_day(to), year_of(to))
            };
            (text, format!("{} days, {} with commits", to - from + 1, a.days))
        }
        _ => ("—".into(), "no commits".into()),
    };
    let figures = [
        ("Commits", a.commits.to_string(), format!("{scope}, {range}")),
        ("Authors", a.authors.len().to_string(), "grouped by email".into()),
        ("Merges", a.merges.to_string(), "included in commits".into()),
        ("Active", active, span),
    ];
    ui.columns(4, |columns| {
        for (ui, (label, value, note)) in columns.iter_mut().zip(figures) {
            Frame::new()
                .fill(PANEL())
                .stroke(Stroke::new(1.0, HAIRLINE()))
                .corner_radius(10)
                .inner_margin(Margin::symmetric(14, 12))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.spacing_mut().item_spacing.y = 3.0;
                    ui.label(RichText::new(label).size(12.0).color(HINT()));
                    ui.add(Label::new(RichText::new(value).font(semibold(21.0))).truncate());
                    ui.add(Label::new(RichText::new(note).size(12.0).color(FAINT())).truncate());
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
                (label.clone(), format!("Week of {label}"), counts.get(&monday).copied().unwrap_or(0))
            })
            .collect();
        return Series { title: "Commits per week", unit: "Week of", buckets };
    }
    let (Some(&(y0, m0, _)), Some(&(y1, m1, _))) = (a.months.first(), a.months.last()) else {
        return Series { title: "Commits per week", unit: "Week of", buckets: Vec::new() };
    };
    let counts: HashMap<(i32, u32), usize> = a.months.iter().map(|&(y, m, n)| ((y, m), n)).collect();
    let span = (y1 - y0) as usize * 12 + m1 as usize + 1 - m0 as usize;
    if span <= 48 {
        let buckets = (0..span)
            .map(|i| {
                let month = m0 as usize - 1 + i;
                let (y, m) = (y0 + (month / 12) as i32, (month % 12) as u32 + 1);
                let label = format!("{} {y}", MONTHS[m as usize - 1]);
                (label.clone(), label, counts.get(&(y, m)).copied().unwrap_or(0))
            })
            .collect();
        return Series { title: "Commits per month", unit: "Month", buckets };
    }
    let buckets = (y0..=y1)
        .map(|y| {
            let n = (1..=12).map(|m| counts.get(&(y, m)).copied().unwrap_or(0)).sum();
            (y.to_string(), y.to_string(), n)
        })
        .collect();
    Series { title: "Commits per year", unit: "Year", buckets }
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
            two_columns(ui, "stats-weeks-table", (series.unit, "Commits"), series.buckets.iter().map(|(axis, _, n)| (axis.clone(), n.to_string())));
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
            response.on_hover_text_at_pointer(format!("{label} · {count} commit{}", if *count == 1 { "" } else { "s" }));
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
        rows.push((format!("{} other authors", a.authors.len() - SHOWN), others));
    }
    let share = |n: usize| (n as f32 / total as f32 * 100.0).round() as usize;
    card(ui, "Authors", "", |ui| {
        if table {
            two_columns(ui, "stats-authors-table", ("Author", "Commits"), rows.iter().map(|(name, n)| (name.clone(), format!("{n} · {}%", share(*n)))));
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
            response.on_hover_text_at_pointer(format!("{name}: {n} commits ({}%)", share(*n)));
        }
    });
}

/// When commits happen: weekday by hour, coloured by the ramp, or a table.
fn heatmap(ui: &mut Ui, a: &CommitActivity, table: bool) {
    const DAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
    card(ui, "When commits happen", "by weekday and hour, in each author's time zone", |ui| {
        if table {
            let rows = DAYS.iter().enumerate().map(|(d, day)| {
                let busiest = (0..24).max_by_key(|&h| (a.heat[d][h], std::cmp::Reverse(h))).unwrap_or(0);
                let note = if a.weekdays[d] == 0 { String::new() } else { format!(" · most at {busiest:02}:00") };
                ((*day).to_owned(), format!("{}{note}", a.weekdays[d]))
            });
            two_columns(ui, "stats-heat-table", ("Weekday", "Commits"), rows);
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
            painter.text(pos2(rect.left(), y + height / 2.0), Align2::LEFT_CENTER, *day, FontId::proportional(11.5), HINT());
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
        painter.text(pos2(x - 6.0, y), Align2::RIGHT_CENTER, "None", FontId::proportional(11.0), FAINT());
        for color in RAMP() {
            painter.rect_filled(Rect::from_min_size(pos2(x, y - 5.0), vec2(14.0, 10.0)), 2.0, color);
            x += 16.0;
        }
        painter.text(pos2(x + 4.0, y), Align2::LEFT_CENTER, "Most", FontId::proportional(11.0), FAINT());
        if let Some((d, h, n)) = hovered {
            response.on_hover_text_at_pointer(format!("{} {h:02}:00 · {n} commit{}", DAYS[d], if n == 1 { "" } else { "s" }));
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
            name: format!("{} smaller entries", rest.len()),
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
    let crumbs: Vec<String> = std::iter::once("Repository".to_owned())
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
        ui.label(RichText::new("· all files at HEAD").size(12.0).color(FAINT()));
        table_toggle(ui, s, widgets);
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
                } else if let Some(clicked) = crate::preview_tab::downloading(ui, preview, "Downloading the files of this commit", "treemap-download-stop", widgets) {
                    stop = clicked;
                } else {
                    pages::loading(ui, "Reading files and history…");
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
                        pages::busy_line(ui, "Reading files and history…");
                    }
                    if let Some(e) = &s.treemap_error {
                        pages::callout(ui, AMBER(), e);
                    }
                    let period = s.period.phrase();
                    card(
                        ui,
                        "Where changes happen",
                        &format!("area is size, colour is commits {period} that changed it · click a folder to zoom in"),
                        |ui| {
                            pages::note(
                                ui,
                                &format!(
                                    "{} files · {} · HEAD {} · {} commits read{}",
                                    folder.files,
                                    size(folder.size),
                                    &map.commit.as_str()[..8],
                                    map.commits,
                                    if map.limited { ", the newest only" } else { "" }
                                ),
                            );
                            if s.table {
                                entries_table(ui, folder);
                            } else {
                                open = treemap(ui, folder, s.zoom.len(), &s.zoom, widgets, period);
                            }
                        },
                    );
                    card(ui, "Most changed files", &format!("{period}, in this folder"), |ui| {
                        let top = s.top.as_ref().map(|(_, files)| files.as_slice()).unwrap_or_default();
                        if top.is_empty() {
                            pages::note(ui, "No file here changed in this period.");
                            return;
                        }
                        let rows = top.iter().map(|(path, changes, bytes)| {
                            [path.clone(), changes.to_string(), size(*bytes)]
                        });
                        three_columns(ui, "stats-top-files", ["File", "Commits", "Size"], rows);
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
            "No files with content here.",
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
                let commits = format!("{} commit{}", cell.changes, if cell.changes == 1 { "" } else { "s" });
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
            format!(" · {} files", cell.files)
        };
        let r = r.on_hover_text_at_pointer(format!(
            "{path}{}\n{}{files} · {} commits {period}{}",
            if folder_cell { "/" } else { "" },
            size(cell.size),
            cell.changes,
            if folder_cell {
                "\nClick to open this folder"
            } else {
                ""
            }
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
    let galley = painter.layout_no_wrap("0 commits".into(), FontId::proportional(11.0), FAINT());
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
        format!("{max} commit{}", if max == 1 { "" } else { "s" }),
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
    three_columns(ui, "stats-entries-table", ["Name", "Commits", "Size"], rows);
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
                    while app
                        .dialog
                        .as_ref()
                        .and_then(|d| d.statistics.as_ref())
                        .is_some_and(State::running)
                    {
                        app.poll(&ctx);
                        assert!(Instant::now() < deadline, "statistics did not finish");
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    frame(&ctx, app, vec![]);
                    frame(&ctx, app, vec![]);
                };
                fn s(app: &Rocket) -> &State {
                    app.dialog.as_ref().unwrap().statistics.as_ref().unwrap()
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
