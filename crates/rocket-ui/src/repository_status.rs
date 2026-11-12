//! Each saved repository's status in the Repository manager, read in the background a
//! few at a time, and Fetch all / Pull all over the repositories a selection lists.
//! Reads are local only. Fetch all follows automatic fetch's rules and Pull all is
//! fast-forward only; neither asks for a sign-in, so a remote that needs one fails.
use crate::{
    app::*,
    backend::{Backend, user_error},
    pages,
    repository_library::{Library, same_path},
    repository_manager::Selection,
    style::*,
};
use eframe::egui::*;
use rocket_git::*;
use serde_json::{Value, json};
use std::{
    collections::{HashMap, VecDeque},
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

/// Status reads at once, and how long each may take.
pub const READS: usize = 4;
pub const READ_LIMIT: Duration = Duration::from_secs(10);
/// Repositories fetched or pulled at once.
pub const RUNS: usize = 2;

/// What the last read of one saved repository found.
pub enum Known {
    Ready(Box<RepositorySummary>),
    /// Its folder is gone.
    Missing,
    /// The read failed or took too long, in words.
    Failed(String),
}
pub struct Entry {
    /// The folder it was read from; a shortcut moved since is not checked yet.
    pub path: PathBuf,
    pub known: Known,
}
struct Read {
    id: u64,
    path: PathBuf,
    started: Instant,
    task: ReadTask<RepositorySummary>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BatchKind {
    Fetch,
    Pull,
}
impl BatchKind {
    pub fn title(self) -> &'static str {
        match self {
            Self::Fetch => "Fetch all",
            Self::Pull => "Pull all",
        }
    }
    fn verb(self) -> &'static str {
        match self {
            Self::Fetch => "Fetching",
            Self::Pull => "Pulling",
        }
    }
    pub fn code(self) -> &'static str {
        match self {
            Self::Fetch => "fetch",
            Self::Pull => "pull",
        }
    }
}

/// What Fetch all or Pull all did with one repository.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Every remote fetched; whether anything new arrived.
    Fetched { changed: bool },
    /// The branch moved forward by this many commits, to `head`.
    Pulled { commits: Option<u64>, head: String },
    /// Nothing to bring in; local commits stay.
    UpToDate,
    /// Left as it was, and why. Never forced.
    Skipped(String),
    Failed(String),
    /// Stop came first.
    Stopped,
}
impl Outcome {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Fetched { .. } => "fetched",
            Self::Pulled { .. } => "pulled",
            Self::UpToDate => "up_to_date",
            Self::Skipped(_) => "skipped",
            Self::Failed(_) => "failed",
            Self::Stopped => "stopped",
        }
    }
    pub fn detail(&self) -> String {
        match self {
            Self::Fetched { changed } => if *changed { "Updated" } else { "Nothing new" }.into(),
            Self::Pulled { commits: Some(n), head } => format!("{} · now at {head}", plural(*n as usize, "commit", "commits")),
            Self::Pulled { commits: None, head } => format!("Now at {head}"),
            Self::UpToDate => "Up to date".into(),
            Self::Skipped(why) | Self::Failed(why) => why.clone(),
            Self::Stopped => "Stopped".into(),
        }
    }
}

enum RunTask {
    Fetch(ReadTask<QuietFetchReport>),
    Pull(WriteTask<(PullReport, Option<u64>)>),
}
struct Run {
    id: u64,
    path: PathBuf,
    task: RunTask,
}
/// Fetch all or Pull all while it runs.
pub struct Batch {
    pub kind: BatchKind,
    pub selection: Selection,
    pub scope: String,
    pub total: usize,
    /// The list order, which the result keeps.
    order: Vec<u64>,
    pending: VecDeque<(u64, PathBuf)>,
    running: Vec<Run>,
    pub outcomes: Vec<(u64, Outcome)>,
    started: Instant,
    pub stopping: bool,
}
impl Batch {
    pub fn done(&self) -> usize {
        self.outcomes.len()
    }
    pub fn progress(&self) -> String {
        if self.stopping {
            format!("Stopping · {} left", self.running.len())
        } else {
            format!("{} · {} of {} done", self.kind.verb(), self.done(), self.total)
        }
    }
}
/// What the last Fetch all or Pull all did, shown for its selection until closed.
pub struct BatchResult {
    pub kind: BatchKind,
    pub selection: Selection,
    pub scope: String,
    pub total: usize,
    pub outcomes: Vec<(u64, Outcome)>,
    pub took: Duration,
    pub stopped: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Good,
    Warn,
    Mute,
}
impl Tone {
    pub fn color(self) -> Color32 {
        match self {
            Self::Good => GREEN(),
            Self::Warn => AMBER(),
            Self::Mute => MUTED(),
        }
    }
    pub fn icon(self) -> Icon {
        match self {
            Self::Good => Icon::Check,
            Self::Warn => Icon::Warning,
            Self::Mute => Icon::Info,
        }
    }
}
impl BatchResult {
    fn count(&self, code: &str) -> usize {
        self.outcomes.iter().filter(|(_, o)| o.code() == code).count()
    }
    /// The strip's short line and the card's longer one, and their tone.
    pub fn summary(&self) -> (String, String, Tone) {
        let (failed, skipped) = (self.count("failed"), self.count("skipped"));
        let tone = if self.stopped {
            Tone::Mute
        } else if failed + skipped > 0 {
            Tone::Warn
        } else {
            Tone::Good
        };
        let parts = |items: &[(usize, &str)]| {
            items
                .iter()
                .filter(|(n, _)| *n > 0)
                .map(|(n, label)| format!("{n} {label}"))
                .collect::<Vec<_>>()
                .join(" · ")
        };
        match self.kind {
            BatchKind::Fetch => {
                let fetched = self.count("fetched");
                let short = if self.stopped {
                    format!("Stopped · {fetched} of {} fetched", self.total)
                } else {
                    [format!("Fetched {fetched}"), parts(&[(failed, "failed"), (skipped, "skipped")])]
                        .into_iter()
                        .filter(|s| !s.is_empty())
                        .collect::<Vec<_>>()
                        .join(" · ")
                };
                let long = parts(&[(fetched, "fetched"), (failed, "failed"), (skipped, "skipped"), (self.count("stopped"), "stopped")]);
                (short, long, tone)
            }
            BatchKind::Pull => {
                let (pulled, current) = (self.count("pulled"), self.count("up_to_date"));
                let short = if self.stopped {
                    format!("Stopped · {pulled} of {} pulled", self.total)
                } else if failed + skipped > 0 {
                    format!("Pulled {pulled} · {}", parts(&[(skipped, "skipped"), (failed, "failed")]))
                } else if current > 0 {
                    format!("Pulled {pulled} · {current} nothing to pull")
                } else {
                    format!("Pulled {pulled}")
                };
                let long = parts(&[
                    (pulled, "pulled"),
                    (skipped, "skipped"),
                    (failed, "failed"),
                    (current, "nothing to pull"),
                    (self.count("stopped"), "stopped"),
                ]);
                (short, long, tone)
            }
        }
    }
    /// Failed and skipped repositories, the ones that need someone.
    pub fn need_you(&self) -> Vec<u64> {
        self.outcomes
            .iter()
            .filter(|(_, o)| matches!(o, Outcome::Failed(_) | Outcome::Skipped(_)))
            .map(|(id, _)| *id)
            .collect()
    }
}

/// The Repository manager's view of each saved repository, and its runs.
#[derive(Default)]
pub struct Status {
    entries: HashMap<u64, Entry>,
    /// The last run's failed fetch per repository, until a later one fetches it.
    fetch_errors: HashMap<u64, (PathBuf, String)>,
    queue: VecDeque<(u64, PathBuf)>,
    /// Every repository queued this visit and its path: each is read once a visit
    /// unless asked again.
    queued: HashMap<u64, PathBuf>,
    reads: Vec<Read>,
    /// This pass: how many were queued and how many finished.
    pass: (usize, usize),
    pub checked: Option<Instant>,
    pub attention_only: bool,
    pub batch: Option<Batch>,
    pub result: Option<BatchResult>,
    visible: bool,
}

/// A details card row: its label, value and colour.
pub(crate) type DetailRow = (&'static str, String, Color32);
/// A details card's note: its colour and text.
pub(crate) type Note = (Color32, String);
/// A row's second line and its two right-hand columns.
pub(crate) struct Row {
    pub branch: Option<String>,
    pub changes: Option<Cell>,
    pub upstream: Option<Cell>,
    /// Draw the tile in red: its folder is gone or it can't be read.
    pub problem: bool,
}
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Cell {
    Pill(String, Color32),
    /// A green check and quiet text: nothing to do.
    Quiet(String),
    /// Quiet italic text: nothing known yet.
    Italic(String),
    /// A spinner and the text.
    Busy(String),
}

impl Status {
    pub fn entry(&self, id: u64, path: &Path) -> Option<&Entry> {
        self.entries.get(&id).filter(|e| same_path(&e.path, path))
    }
    fn summary(&self, id: u64, path: &Path) -> Option<&RepositorySummary> {
        match &self.entry(id, path)?.known {
            Known::Ready(summary) => Some(summary),
            _ => None,
        }
    }
    pub fn fetch_error(&self, id: u64, path: &Path) -> Option<&str> {
        self.fetch_errors
            .get(&id)
            .filter(|(p, _)| same_path(p, path))
            .map(|(_, e)| e.as_str())
    }
    pub fn reading(&self, id: u64) -> bool {
        self.reads.iter().any(|r| r.id == id)
    }
    pub fn checking(&self) -> bool {
        !self.queue.is_empty() || !self.reads.is_empty()
    }
    /// "Checking 3 of 9" while a pass runs.
    pub fn progress(&self) -> Option<String> {
        self.checking()
            .then(|| format!("Checking {} of {}", (self.pass.1 + 1).min(self.pass.0.max(1)), self.pass.0.max(1)))
    }
    /// Changes, commits to push or pull, an operation in progress, a failed fetch, or a
    /// folder that is gone or can't be read.
    pub fn attention(&self, id: u64, path: &Path) -> bool {
        self.fetch_error(id, path).is_some()
            || self.entry(id, path).is_some_and(|e| match &e.known {
                Known::Ready(s) => {
                    s.changes.is_some_and(|c| c.total() > 0)
                        || s.ahead_behind.is_some_and(|(a, b)| a > 0 || b > 0)
                        || !s.operation.is_idle()
                }
                Known::Missing | Known::Failed(_) => true,
            })
    }
    /// Whether Pull all has anything to try: one listed repository with an upstream or
    /// not read yet.
    pub fn pullable<'a>(&self, rows: impl IntoIterator<Item = (u64, &'a Path)>) -> bool {
        rows.into_iter().any(|(id, path)| match self.entry(id, path).map(|e| &e.known) {
            None => true,
            Some(Known::Ready(s)) => s.upstream.is_some(),
            Some(_) => false,
        })
    }

    /// Queue the listed repositories this visit hasn't read yet, in list order.
    pub fn want<'a>(&mut self, rows: impl IntoIterator<Item = (u64, &'a Path)>) {
        for (id, path) in rows {
            if !self.queued.get(&id).is_some_and(|p| same_path(p, path)) {
                self.enqueue(id, path.to_path_buf(), false);
            }
        }
    }
    /// Read these again now.
    pub fn check_again<'a>(&mut self, rows: impl IntoIterator<Item = (u64, &'a Path)>) {
        for (id, path) in rows {
            self.enqueue(id, path.to_path_buf(), false);
        }
    }
    fn enqueue(&mut self, id: u64, path: PathBuf, front: bool) {
        self.queued.insert(id, path.clone());
        if let Some(waiting) = self.queue.iter_mut().find(|(q, _)| *q == id) {
            waiting.1 = path;
            return;
        }
        if !self.checking() {
            self.pass = (0, 0);
        }
        self.pass.0 += 1;
        if front {
            self.queue.push_front((id, path));
        } else {
            self.queue.push_back((id, path));
        }
    }
    /// An Open that failed: the same as a read that failed.
    pub(crate) fn record_failure(&mut self, id: u64, path: &Path, error: &str) {
        let known = if matches!(path.try_exists(), Ok(false)) { Known::Missing } else { Known::Failed(error.into()) };
        self.entries.insert(id, Entry { path: path.to_path_buf(), known });
    }
    /// The page closed: stop reading, and read again on the next visit. Results stay.
    fn close(&mut self) {
        self.reads.clear(); // A dropped read cancels its Git command.
        self.queue.clear();
        self.queued.clear();
        self.pass = (0, 0);
    }
    /// Quitting: stop reading, and stop the run as Exit stops any write: nothing new
    /// starts and running Git is cancelled. The run stays until its Git reports back, so
    /// Rocket closes only after that.
    pub fn quit(&mut self) {
        self.close();
        if let Some(batch) = &mut self.batch {
            batch.stopping = true;
            for run in &batch.running {
                match &run.task {
                    RunTask::Fetch(task) => task.cancel(),
                    RunTask::Pull(task) => task.cancel(),
                }
            }
        }
    }

    pub(crate) fn row(&self, id: u64, path: &Path) -> Row {
        let mut row = match self.entry(id, path) {
            None => Row {
                branch: None,
                changes: None,
                upstream: Some(if self.reading(id) {
                    Cell::Busy("Checking".into())
                } else {
                    Cell::Italic("Not checked yet".into())
                }),
                problem: false,
            },
            Some(Entry { known: Known::Missing, .. }) => Row {
                branch: None,
                changes: None,
                upstream: Some(Cell::Pill("Folder missing".into(), RED())),
                problem: true,
            },
            Some(Entry { known: Known::Failed(_), .. }) => Row {
                branch: None,
                changes: None,
                upstream: Some(Cell::Pill("Couldn't check".into(), RED())),
                problem: true,
            },
            Some(Entry { known: Known::Ready(s), .. }) => Row {
                branch: Some(branch_text(&s.head)),
                changes: changes_cell(s),
                upstream: Some(if self.fetch_error(id, path).is_some() {
                    Cell::Pill("Fetch failed".into(), RED())
                } else {
                    upstream_cell(s)
                }),
                problem: false,
            },
        };
        if let Some(batch) = &self.batch {
            if batch.running.iter().any(|r| r.id == id) {
                row.upstream = Some(Cell::Busy(batch.kind.verb().into()));
            } else if batch.pending.iter().any(|(p, _)| *p == id) {
                row.upstream = Some(Cell::Pill("Waiting".into(), pages::NEUTRAL()));
            }
        }
        row
    }

    /// The details card's status rows, and the one note that matters most.
    pub(crate) fn details(&self, id: u64, path: &Path) -> (Vec<DetailRow>, Option<Note>) {
        let Some(entry) = self.entry(id, path) else {
            let text = if self.reading(id) { "Checking" } else { "Not checked yet" };
            return (vec![("Status", text.into(), HINT())], None);
        };
        let s = match &entry.known {
            Known::Missing => {
                return (vec![], Some((RED(), "This folder is gone. Edit its location, or remove it from the library.".into())));
            }
            Known::Failed(why) => return (vec![("Status", "Couldn't check".into(), RED())], Some((RED(), why.clone()))),
            Known::Ready(s) => s,
        };
        let fetch_error = self.fetch_error(id, path);
        let branch = match (&s.head, &s.upstream) {
            (Head::Branch { name, .. }, Some(upstream)) => format!("{} → {}", display(name), display(upstream)),
            (head, _) => branch_text(head),
        };
        let at_fetch = if fetch_error.is_some() { " at the last fetch" } else { "" };
        let (upstream, upstream_color) = match (&s.head, &s.upstream, s.ahead_behind) {
            (Head::Detached { .. }, ..) | (_, None, _) => ("None".into(), MUTED()),
            (_, Some(_), None) => ("Gone from its remote".into(), MUTED()),
            (_, Some(_), Some((0, 0))) => (format!("Up to date{at_fetch}"), TEXT()),
            (_, Some(_), Some((a, 0))) => (format!("↑{a} ahead{at_fetch}"), BLUE()),
            (_, Some(_), Some((0, b))) => (format!("↓{b} behind{at_fetch}"), BLUE()),
            (_, Some(_), Some((a, b))) => (format!("↑{a} ahead · ↓{b} behind{at_fetch}"), AMBER()),
        };
        let changes = s.changes.unwrap_or_default();
        let (changed, changed_color) = if let Some(label) = operation_label(&s.operation) {
            (format!("{label} in progress"), AMBER())
        } else if changes.total() > 0 {
            let mut parts = Vec::new();
            if changes.tracked > 0 {
                parts.push(format!("{} changed", changes.tracked));
            }
            if changes.new > 0 {
                parts.push(format!("{} new", changes.new));
            }
            (format!("{} uncommitted: {}", changes.total(), parts.join(", ")), AMBER())
        } else {
            ("None".into(), MUTED())
        };
        let fetched = match s.last_fetch {
            Some(time) => (since(time), TEXT()),
            None => ("Never".into(), MUTED()),
        };
        let rows = vec![
            ("Branch", branch, TEXT()),
            ("Upstream", upstream, upstream_color),
            ("Changes", changed, changed_color),
            ("Last fetch", fetched.0, fetched.1),
        ];
        let behind = s.ahead_behind.map_or(0, |(_, b)| b);
        let note = if let Some(error) = fetch_error {
            Some((RED(), format!("Fetch failed: {error}")))
        } else if s.ahead_behind.is_some_and(|(a, b)| a > 0 && b > 0) {
            Some((AMBER(), "Diverged. Pull all leaves it for you to merge or rebase.".into()))
        } else if let Some(label) = operation_label(&s.operation) {
            Some((AMBER(), format!("{label} is in progress. Pull all skips it.")))
        } else if changes.tracked > 0 && behind > 0 {
            Some((AMBER(), "Pull all skips it while it has local changes.".into()))
        } else {
            None
        };
        (rows, note)
    }

    /// Moves the reads on: collects finished ones, gives up on slow ones and starts
    /// queued ones, `READS` at a time. Repositories in `busy` wait for their run.
    fn pump(&mut self, backend: &Backend, busy: &[u64], ctx: &Context) {
        let now = Instant::now();
        let mut done = Vec::new();
        self.reads.retain_mut(|read| {
            let known = if let Some(result) = read.task.try_finish() {
                known(&read.path, result)
            } else if now.duration_since(read.started) >= READ_LIMIT {
                read.task.cancel();
                Known::Failed(format!("Took longer than {} s.", READ_LIMIT.as_secs()))
            } else {
                return true;
            };
            done.push((read.id, read.path.clone(), known));
            false
        });
        for (id, path, known) in done {
            self.pass.1 += 1;
            self.entries.insert(id, Entry { path, known });
        }
        let mut deferred = Vec::new();
        while self.reads.len() < READS {
            let Some((id, path)) = self.queue.pop_front() else {
                break;
            };
            if busy.contains(&id) {
                deferred.push((id, path));
                continue;
            }
            let (worker, target, wake) = (backend.clone(), path.clone(), ctx.clone());
            match ReadTask::spawn_with_wake(
                move |cancel| worker.repository_summary(&target, READ_LIMIT, &cancel),
                move || wake.request_repaint(),
            ) {
                Ok(task) => self.reads.push(Read { id, path, started: now, task }),
                Err(error) => {
                    self.pass.1 += 1;
                    self.entries.insert(id, Entry { path, known: Known::Failed(user_error(&error)) });
                }
            }
        }
        self.queue.extend(deferred);
        if !self.checking() && self.pass.0 > 0 {
            self.checked = Some(now);
            self.pass = (0, 0);
        }
    }

    /// Moves Fetch all or Pull all on, `RUNS` at a time. Returns the folders whose open
    /// tabs should refresh, and whether it just finished.
    fn pump_batch(&mut self, backend: &Backend, tags: FetchTags, busy: &[PathBuf], ctx: &Context) -> (Vec<PathBuf>, bool) {
        let Some(mut batch) = self.batch.take() else {
            return (Vec::new(), false);
        };
        let mut refresh = Vec::new();
        let mut index = 0;
        while index < batch.running.len() {
            let run = &mut batch.running[index];
            let finished = match &mut run.task {
                RunTask::Fetch(task) => task.try_finish().map(|result| {
                    let fetched = result.as_ref().ok().map(|report| report.results.iter().all(|(_, r)| r.is_ok()));
                    (fetch_outcome(result), fetched)
                }),
                RunTask::Pull(task) => task.try_finish().map(|result| {
                    let fetched = result.as_ref().ok().map(|(report, _)| report.fetch.is_ok());
                    let tracked = match self.entries.get(&run.id).map(|e| &e.known) {
                        Some(Known::Ready(s)) => s.changes.map(|c| c.tracked),
                        _ => None,
                    };
                    (pull_outcome(result, tracked), fetched)
                }),
            };
            let Some((outcome, fetched)) = finished else {
                index += 1;
                continue;
            };
            let run = batch.running.remove(index);
            match (&outcome, fetched) {
                (Outcome::Failed(why), Some(false)) => {
                    self.fetch_errors.insert(run.id, (run.path.clone(), why.clone()));
                }
                (_, Some(true)) => {
                    self.fetch_errors.remove(&run.id);
                }
                _ => {}
            }
            if fetched.is_some() {
                refresh.push(run.path.clone());
            }
            self.enqueue(run.id, run.path, true);
            batch.outcomes.push((run.id, outcome));
        }
        if batch.stopping {
            for (id, _) in batch.pending.drain(..) {
                batch.outcomes.push((id, Outcome::Stopped));
            }
            // A fetch stops at once; a pull already running finishes, so a fast-forward
            // is never cut short.
            for run in &batch.running {
                if let RunTask::Fetch(task) = &run.task {
                    task.cancel();
                }
            }
        }
        while !batch.stopping && batch.running.len() < RUNS {
            let Some((id, path)) = batch.pending.pop_front() else {
                break;
            };
            if matches!(path.try_exists(), Ok(false)) {
                self.entries.insert(id, Entry { path, known: Known::Missing });
                batch.outcomes.push((id, Outcome::Skipped("Folder missing".into())));
                continue;
            }
            if busy.iter().any(|p| same_path(p, &path)) {
                batch.outcomes.push((id, Outcome::Skipped("Busy in its tab".into())));
                continue;
            }
            let (worker, target, wake) = (backend.clone(), path.clone(), ctx.clone());
            let task = match batch.kind {
                BatchKind::Fetch => ReadTask::spawn_with_wake(
                    move |cancel| worker.fetch_all_listed(&target, tags, &cancel),
                    move || wake.request_repaint(),
                )
                .map(RunTask::Fetch),
                BatchKind::Pull => {
                    WriteTask::spawn(move |cancel| worker.pull_ff_only_quietly(&target, &cancel)).map(RunTask::Pull)
                }
            };
            match task {
                Ok(task) => batch.running.push(Run { id, path, task }),
                Err(error) => batch.outcomes.push((id, Outcome::Failed(user_error(&error)))),
            }
        }
        if !batch.pending.is_empty() || !batch.running.is_empty() {
            self.batch = Some(batch);
            return (refresh, false);
        }
        let order = &batch.order;
        batch.outcomes.sort_by_key(|(id, _)| order.iter().position(|o| o == id));
        self.result = Some(BatchResult {
            kind: batch.kind,
            selection: batch.selection,
            scope: batch.scope,
            total: batch.total,
            outcomes: batch.outcomes,
            took: batch.started.elapsed(),
            stopped: batch.stopping,
        });
        (refresh, true)
    }

    /// For automation: every saved repository's status, the run and its result.
    pub fn mcp_state(&self, library: &Library) -> Value {
        let repositories: Vec<Value> = library
            .repositories
            .iter()
            .map(|r| {
                let entry = self.entry(r.id, &r.path);
                let state = match entry.map(|e| &e.known) {
                    None if self.reading(r.id) => "checking",
                    None => "unchecked",
                    Some(Known::Ready(_)) => "ready",
                    Some(Known::Missing) => "missing",
                    Some(Known::Failed(_)) => "failed",
                };
                let s = self.summary(r.id, &r.path);
                json!({
                    "id": r.id,
                    "state": state,
                    "branch": s.map(|s| branch_text(&s.head)),
                    "upstream": s.and_then(|s| s.upstream.as_deref().map(display)),
                    "ahead": s.and_then(|s| s.ahead_behind.map(|(a, _)| a)),
                    "behind": s.and_then(|s| s.ahead_behind.map(|(_, b)| b)),
                    "changes": s.and_then(|s| s.changes.map(SummaryChanges::total)),
                    "tracked": s.and_then(|s| s.changes.map(|c| c.tracked)),
                    "operation": s.and_then(|s| operation_label(&s.operation)),
                    "fetch_error": self.fetch_error(r.id, &r.path),
                    "attention": self.attention(r.id, &r.path),
                    "row": self.row(r.id, &r.path).upstream.map(|c| match c {
                        Cell::Pill(text, _) | Cell::Quiet(text) | Cell::Italic(text) | Cell::Busy(text) => text,
                    }),
                })
            })
            .collect();
        json!({
            "checking": self.checking(),
            "checked": self.checked.is_some(),
            "attention_only": self.attention_only,
            "repositories": repositories,
            "batch": self.batch.as_ref().map(|b| json!({
                "kind": b.kind.code(), "total": b.total, "done": b.done(),
                "running": b.running.len(), "stopping": b.stopping,
            })),
            "result": self.result.as_ref().map(|r| json!({
                "kind": r.kind.code(), "scope": r.scope, "total": r.total, "stopped": r.stopped,
                "outcomes": r.outcomes.iter().map(|(id, o)| json!({"id": id, "outcome": o.code(), "detail": o.detail()})).collect::<Vec<_>>(),
            })),
        })
    }
}

fn known(path: &Path, result: Result<RepositorySummary>) -> Known {
    match result {
        Ok(summary) => Known::Ready(Box::new(summary)),
        Err(_) if matches!(path.try_exists(), Ok(false)) => Known::Missing,
        Err(error) => Known::Failed(crate::repository_manager::check_error(&error)),
    }
}

pub(crate) fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}
fn short(id: &ObjectId) -> String {
    id.as_str().chars().take(7).collect()
}
/// "main", "Detached at 3f2a9c1", "main, no commits yet".
pub(crate) fn branch_text(head: &Head) -> String {
    match head {
        Head::Branch { name, .. } => display(name),
        Head::Detached { target } => format!("Detached at {}", short(target)),
        Head::Unborn { branch } => format!("{}, no commits yet", display(branch)),
    }
}
/// The operation in progress, as a row's pill says it.
pub(crate) fn operation_label(operation: &OperationState) -> Option<&'static str> {
    if operation.is_idle() {
        None
    } else if operation.rebase {
        Some("Rebasing")
    } else if !operation.merge_heads.is_empty() || operation.merge_autostash.is_some() {
        Some("Merging")
    } else if operation.cherry_pick {
        Some("Cherry-picking")
    } else if operation.revert {
        Some("Reverting")
    } else if operation.mail_import {
        Some("Importing")
    } else if operation.bisect {
        Some("Bisecting")
    } else {
        Some("In progress")
    }
}
fn changes_cell(s: &RepositorySummary) -> Option<Cell> {
    if let Some(label) = operation_label(&s.operation) {
        return Some(Cell::Pill(label.into(), AMBER()));
    }
    let n = s.changes.map_or(0, SummaryChanges::total);
    (n > 0).then(|| Cell::Pill(plural(n, "change", "changes"), AMBER()))
}
fn upstream_cell(s: &RepositorySummary) -> Cell {
    match (&s.head, &s.upstream, s.ahead_behind) {
        (Head::Detached { .. }, ..) => Cell::Pill("No branch".into(), pages::NEUTRAL()),
        (_, None, _) => Cell::Pill("No upstream".into(), pages::NEUTRAL()),
        (_, Some(_), None) => Cell::Pill("Upstream gone".into(), pages::NEUTRAL()),
        (_, Some(_), Some((0, 0))) => Cell::Quiet("Up to date".into()),
        (_, Some(_), Some((a, 0))) => Cell::Pill(format!("↑{a}"), BLUE()),
        (_, Some(_), Some((0, b))) => Cell::Pill(format!("↓{b}"), BLUE()),
        (_, Some(_), Some((a, b))) => Cell::Pill(format!("↑{a} ↓{b}"), AMBER()),
    }
}
/// "just now", "12 min ago", "3 days ago".
pub(crate) fn since(time: SystemTime) -> String {
    let age = SystemTime::now().duration_since(time).map_or(0, |d| d.as_secs());
    let unix = time.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
    ago(age, unix)
}
/// "Checked just now", "Checked 4 min ago".
pub(crate) fn checked_text(at: Instant) -> String {
    format!("Checked {}", since(SystemTime::now() - at.elapsed()))
}

/// A short reason for a failed fetch: sign-in, access and connection problems in a
/// few words, anything else as Rocket words it elsewhere.
fn fetch_reason(error: &Error) -> String {
    use FetchFailure::*;
    let Some(failure) = error.fetch_failure() else {
        return user_error(error);
    };
    match failure {
        CredentialsUnavailable | SshKeyUnavailable | SshKeyUnlockFailed | SshAgentFailure => "Needs a sign-in".into(),
        AuthenticationRejected | SshAuthenticationRejected | SsoRequired => "Sign-in was refused".into(),
        AccessDenied => "Access denied".into(),
        RepositoryUnavailable => "No repository at that address".into(),
        ProxyAuthenticationRequired => "The proxy needs a sign-in".into(),
        TlsVerificationFailed => "Certificate check failed".into(),
        SshHostKeyChanged | SshHostVerificationFailed => "Host key check failed".into(),
        NameResolutionFailed | ConnectionFailed | ConnectionTimedOut | ConnectionInterrupted => "Couldn't connect".into(),
        _ => failure.summary().trim_end_matches('.').into(),
    }
}
fn fetch_outcome(result: Result<QuietFetchReport>) -> Outcome {
    match result {
        Ok(report) if report.results.is_empty() => Outcome::Skipped("No remotes".into()),
        Ok(report) => {
            let failed: Vec<String> = report
                .results
                .iter()
                .filter_map(|(name, r)| r.as_ref().err().map(|e| format!("{}: {}", display(name), fetch_reason(e))))
                .collect();
            if failed.is_empty() {
                Outcome::Fetched { changed: report.changed() }
            } else {
                Outcome::Failed(failed.join("; "))
            }
        }
        Err(Error::Cancelled) => Outcome::Stopped,
        Err(error) => Outcome::Failed(user_error(&error)),
    }
}
/// `tracked`: the changed tracked files the last read counted, for the skip reason.
fn pull_outcome(result: Result<(PullReport, Option<u64>)>, tracked: Option<usize>) -> Outcome {
    let (report, commits) = match result {
        Ok(done) => done,
        Err(Error::Pull(failure)) => {
            return match failure {
                PullFailure::DirtyWorktree => Outcome::Skipped(match tracked {
                    Some(n) if n > 0 => format!("{} uncommitted", plural(n, "change", "changes")),
                    _ => "Uncommitted changes".into(),
                }),
                PullFailure::OperationInProgress => Outcome::Skipped("A merge or rebase is in progress".into()),
                PullFailure::DetachedHead => Outcome::Skipped("No branch checked out".into()),
                PullFailure::UnbornHead => Outcome::Skipped("No commits yet".into()),
                PullFailure::MissingUpstream => Outcome::Skipped("No upstream".into()),
                PullFailure::UnsupportedUpstream => Outcome::Skipped("Its upstream isn't a remote branch".into()),
                other => Outcome::Failed(user_error(&Error::Pull(other))),
            };
        }
        Err(Error::Cancelled) => return Outcome::Stopped,
        Err(error) => return Outcome::Failed(user_error(&error)),
    };
    match (&report.fetch, report.update) {
        (Err(Error::Cancelled), _) => Outcome::Stopped,
        (Err(error), _) => Outcome::Failed(fetch_reason(error)),
        (Ok(_), None) => Outcome::Failed("Couldn't read what the fetch brought".into()),
        (Ok(_), Some(Ok(update))) => match update.kind {
            PullUpdateKind::UpToDate => Outcome::UpToDate,
            PullUpdateKind::FastForward if update.verification.is_ok() => {
                Outcome::Pulled { commits, head: short(&update.fetched) }
            }
            PullUpdateKind::FastForward => Outcome::Failed("Pulled, but the new commit couldn't be confirmed".into()),
        },
        (Ok(_), Some(Err(Error::Pull(PullFailure::Diverged)))) => Outcome::Skipped("Diverged".into()),
        (Ok(_), Some(Err(error))) => Outcome::Failed(user_error(&error)),
    }
}

/// A filter chip: its label and count, an amber dot when asked, the accent outline
/// when on.
pub(crate) fn chip(ui: &mut Ui, label: &str, count: usize, on: bool, dot: bool) -> Response {
    let font = FontId::proportional(12.5);
    let (ink, number) = if on { (BLUE(), BLUE()) } else { (MUTED(), FAINT()) };
    let text = ui.painter().layout_no_wrap(label.into(), font.clone(), ink);
    let figure = ui.painter().layout_no_wrap(count.to_string(), font, number);
    let lead = if dot { 13.0 } else { 0.0 };
    let size = vec2(20.0 + lead + text.size().x + 6.0 + figure.size().x, 26.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let hot = ui.is_enabled() && (response.hovered() || response.has_focus());
    let (fill, stroke) = if on {
        (BLUE().gamma_multiply(0.10), BLUE())
    } else if hot {
        (HOVER(), EDGE_HOT())
    } else {
        (Color32::TRANSPARENT, BORDER())
    };
    let painter = ui.painter();
    painter.rect(rect, 13.0, fill, Stroke::new(1.0, stroke), StrokeKind::Inside);
    let mut x = rect.left() + 10.0;
    if dot {
        painter.circle_filled(pos2(x + 3.5, rect.center().y), 3.5, AMBER());
        x += lead;
    }
    let (text_width, figure_height) = (text.size().x, figure.size().y);
    painter.galley(pos2(x, rect.center().y - text.size().y / 2.0), text, ink);
    painter.galley(pos2(x + text_width + 6.0, rect.center().y - figure_height / 2.0), figure, number);
    response.widget_info(|| WidgetInfo::selected(Role::Button, ui.is_enabled(), on, label));
    response
}

/// A framed button with an icon before its label, such as Fetch all.
pub(crate) fn icon_button(ui: &mut Ui, image: Icon, label: &str, enabled: bool) -> Response {
    let font = FontId::proportional(13.0);
    let width = ui.painter().layout_no_wrap(label.into(), font.clone(), TEXT()).size().x;
    let response = ui.add_enabled(enabled, Button::new("").min_size(vec2(width + 40.0, CONTROL)));
    let rect = response.rect;
    let ink = if enabled { TEXT() } else { MUTED() };
    icon(ui.painter(), Rect::from_center_size(pos2(rect.left() + 16.0, rect.center().y), vec2(14.0, 14.0)), image, ink);
    ui.painter().text(pos2(rect.left() + 29.0, rect.center().y), Align2::LEFT_CENTER, label, font, ink);
    response.widget_info(|| WidgetInfo::labeled(Role::Button, enabled, label));
    response
}

/// One right-hand column of a row, its content against the right edge.
pub(crate) fn paint_cell(ui: &Ui, column: Rect, cell: &Cell) {
    let painter = ui.painter().with_clip_rect(column.intersect(ui.clip_rect()));
    let y = column.center().y;
    match cell {
        Cell::Pill(text, color) => {
            let arrows = text.starts_with('↑') || text.starts_with('↓');
            let galley = painter.layout_no_wrap(text.clone(), semibold(if arrows { 12.5 } else { 11.0 }), *color);
            let pad = if arrows { 9.0 } else { 8.0 };
            let pill = Rect::from_min_max(
                pos2(column.right() - galley.size().x - 2.0 * pad, y - 10.0),
                pos2(column.right(), y + 10.0),
            );
            painter.rect_filled(pill, 10.0, color.gamma_multiply(0.14));
            painter.galley(pos2(pill.left() + pad, y - galley.size().y / 2.0), galley, *color);
        }
        Cell::Quiet(text) => {
            let galley = painter.layout_no_wrap(text.clone(), FontId::proportional(12.0), MUTED());
            let left = column.right() - galley.size().x;
            icon(&painter, Rect::from_center_size(pos2(left - 11.0, y), vec2(12.0, 12.0)), Icon::Check, GREEN());
            painter.galley(pos2(left, y - galley.size().y / 2.0), galley, MUTED());
        }
        Cell::Italic(text) => {
            let mut job = text::LayoutJob::default();
            job.append(text, 0.0, TextFormat { font_id: FontId::proportional(12.0), color: HINT(), italics: true, ..Default::default() });
            let galley = painter.layout_job(job);
            painter.galley(pos2(column.right() - galley.size().x, y - galley.size().y / 2.0), galley, HINT());
        }
        Cell::Busy(text) => {
            let galley = painter.layout_no_wrap(text.clone(), FontId::proportional(12.0), BLUE());
            let left = column.right() - galley.size().x;
            Spinner::new()
                .size(11.0)
                .color(BLUE())
                .paint_at(ui, Rect::from_center_size(pos2(left - 11.0, y), vec2(11.0, 11.0)));
            painter.galley(pos2(left, y - galley.size().y / 2.0), galley, BLUE());
        }
    }
}

impl Rocket {
    /// The selection's repositories in list order, ignoring the list's filters: what
    /// Fetch all and Pull all act on.
    pub fn manager_batch_rows(&self) -> Vec<(u64, PathBuf)> {
        crate::repository_manager::selection_repositories(&self.library, self.manager.selection, self.manager.include_nested)
            .map_or_else(|_| Vec::new(), |rows| rows.into_iter().map(|r| (r.id, r.path.clone())).collect())
    }
    /// The last result's repositories that need someone and whose folders exist: what
    /// Open N in tabs opens.
    pub fn manager_result_paths(&self) -> Vec<(u64, PathBuf)> {
        let Some(result) = &self.manager.status.result else {
            return Vec::new();
        };
        result
            .need_you()
            .into_iter()
            .filter_map(|id| self.library.repositories.iter().find(|r| r.id == id))
            .filter(|r| !matches!(r.path.try_exists(), Ok(false)))
            .map(|r| (r.id, r.path.clone()))
            .collect()
    }
    pub(crate) fn start_batch(&mut self, kind: BatchKind) {
        if self.manager.running() || self.write.is_some() || self.quitting || self.backend.is_none() {
            return;
        }
        let rows = self.manager_batch_rows();
        if rows.is_empty() {
            return;
        }
        let scope = crate::repository_manager::selection_title(&self.library, self.manager.selection);
        let status = &mut self.manager.status;
        status.result = None;
        status.batch = Some(Batch {
            kind,
            selection: self.manager.selection,
            scope,
            total: rows.len(),
            order: rows.iter().map(|(id, _)| *id).collect(),
            pending: rows.into(),
            running: Vec::new(),
            outcomes: Vec::new(),
            started: Instant::now(),
            stopping: false,
        });
        self.manager.clear_result();
        if let Some(d) = &mut self.dialog {
            d.flag = true;
        }
    }
    pub(crate) fn stop_batch(&mut self) {
        if let Some(batch) = &mut self.manager.status.batch {
            batch.stopping = true;
        }
    }
    pub(crate) fn poll_repository_status(&mut self, ctx: &Context) {
        if self.quitting {
            self.manager.status.quit();
            // Cancelled runs still report back; working() keeps Rocket open until they do.
            if let Some(backend) = self.backend.clone() {
                self.manager.status.pump_batch(&backend, self.general.fetch_tags, &[], ctx);
            }
            return;
        }
        let visible = self.page() == Some(crate::pages::Page::RepositoryManager);
        let status = &mut self.manager.status;
        if !visible && status.batch.is_none() {
            if status.visible {
                status.close();
            }
            status.visible = false;
            return;
        }
        status.visible = visible;
        let Some(backend) = self.backend.clone() else {
            return;
        };
        // A repository whose tab is writing is left alone.
        let writing: Vec<PathBuf> = self
            .write
            .as_ref()
            .and_then(|w| w.owner)
            .and_then(|owner| self.tabs.iter().find(|t| t.id == owner))
            .map(|t| vec![t.path.clone()])
            .unwrap_or_default();
        let (refresh, finished) = self.manager.status.pump_batch(&backend, self.general.fetch_tags, &writing, ctx);
        let busy: Vec<u64> = self
            .manager
            .status
            .batch
            .as_ref()
            .map(|b| b.running.iter().map(|r| r.id).collect())
            .unwrap_or_default();
        self.manager.status.pump(&backend, &busy, ctx);
        let tabs: Vec<u64> = self
            .tabs
            .iter()
            .filter(|t| refresh.iter().any(|p| same_path(p, &t.path)))
            .map(|t| t.id)
            .collect();
        for id in tabs {
            self.refresh(id);
        }
        if finished && let Some(d) = &mut self.dialog {
            d.flag = false;
        }
        if self.manager.status.checking() || self.manager.status.batch.is_some() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary(ahead_behind: Option<(u64, u64)>, changes: SummaryChanges) -> RepositorySummary {
        RepositorySummary {
            head: Head::Branch { name: b"main".to_vec(), target: ObjectId::parse("0123456789abcdef0123456789abcdef01234567").unwrap() },
            upstream: ahead_behind.map(|_| b"origin/main".to_vec()),
            ahead_behind,
            changes: Some(changes),
            operation: OperationState::default(),
            last_commit: None,
            last_fetch: None,
        }
    }
    fn ready(status: &mut Status, id: u64, s: RepositorySummary) {
        status.entries.insert(id, Entry { path: PathBuf::from(format!("C:\\repo-{id}")), known: Known::Ready(Box::new(s)) });
    }
    fn path(id: u64) -> PathBuf {
        PathBuf::from(format!("C:\\repo-{id}"))
    }

    // Each state reads as the mock shows it, and only real problems need attention.
    #[test]
    fn rows_read_each_state_and_attention_follows_them() {
        let mut s = Status::default();
        let none = SummaryChanges::default();
        ready(&mut s, 1, summary(Some((0, 0)), none));
        ready(&mut s, 2, summary(Some((0, 0)), SummaryChanges { tracked: 2, new: 1 }));
        ready(&mut s, 3, summary(Some((2, 0)), none));
        ready(&mut s, 4, summary(Some((0, 4)), none));
        ready(&mut s, 5, summary(Some((1, 3)), none));
        ready(&mut s, 6, summary(None, none));
        s.entries.insert(7, Entry { path: path(7), known: Known::Missing });
        s.entries.insert(8, Entry { path: path(8), known: Known::Failed("Took longer than 10 s.".into()) });
        let upstream = |s: &Status, id| s.row(id, &path(id)).upstream.unwrap();
        assert_eq!(upstream(&s, 1), Cell::Quiet("Up to date".into()));
        assert_eq!(s.row(2, &path(2)).changes, Some(Cell::Pill("3 changes".into(), AMBER())));
        assert_eq!(upstream(&s, 3), Cell::Pill("↑2".into(), BLUE()));
        assert_eq!(upstream(&s, 4), Cell::Pill("↓4".into(), BLUE()));
        assert_eq!(upstream(&s, 5), Cell::Pill("↑1 ↓3".into(), AMBER()));
        assert_eq!(upstream(&s, 6), Cell::Pill("No upstream".into(), pages::NEUTRAL()));
        assert_eq!(upstream(&s, 7), Cell::Pill("Folder missing".into(), RED()));
        assert!(s.row(7, &path(7)).problem);
        assert_eq!(upstream(&s, 8), Cell::Pill("Couldn't check".into(), RED()));
        assert_eq!(upstream(&s, 9), Cell::Italic("Not checked yet".into()));
        // A moved shortcut is not checked yet at its new folder.
        assert_eq!(s.row(1, Path::new("D:\\elsewhere")).upstream, Some(Cell::Italic("Not checked yet".into())));
        let attention: Vec<u64> = (1..=9).filter(|id| s.attention(*id, &path(*id))).collect();
        assert_eq!(attention, [2, 3, 4, 5, 7, 8]);
        s.fetch_errors.insert(1, (path(1), "origin: Needs a sign-in".into()));
        assert_eq!(upstream(&s, 1), Cell::Pill("Fetch failed".into(), RED()));
        assert!(s.attention(1, &path(1)));
        let (rows, note) = s.details(1, &path(1));
        assert_eq!(rows[1].1, "Up to date at the last fetch");
        assert_eq!(note, Some((RED(), "Fetch failed: origin: Needs a sign-in".into())));
        let (rows, note) = s.details(5, &path(5));
        assert_eq!(rows[0].1, "main → origin/main");
        assert_eq!(rows[1].1, "↑1 ahead · ↓3 behind");
        assert!(note.unwrap().1.starts_with("Diverged"));
        assert_eq!(s.details(2, &path(2)).0[2].1, "3 uncommitted: 2 changed, 1 new");
        // Pull all has nothing to try when nothing listed has an upstream.
        assert!(!s.pullable([(6, path(6).as_path()), (7, path(7).as_path())]));
        assert!(s.pullable([(6, path(6).as_path()), (9, path(9).as_path())]));
    }

    // Reads are queued once a visit in list order, a pass counts what it read, and a
    // forced re-read goes first.
    #[test]
    fn reads_queue_once_a_visit_and_count_their_pass() {
        let mut s = Status::default();
        let rows = [(1, path(1)), (2, path(2)), (3, path(3))];
        s.want(rows.iter().map(|(id, p)| (*id, p.as_path())));
        s.want(rows.iter().map(|(id, p)| (*id, p.as_path())));
        assert_eq!(s.queue.iter().map(|(id, _)| *id).collect::<Vec<_>>(), [1, 2, 3]);
        assert_eq!(s.progress().as_deref(), Some("Checking 1 of 3"));
        s.enqueue(3, path(3), true);
        s.enqueue(4, path(4), true);
        assert_eq!(s.queue.iter().map(|(id, _)| *id).collect::<Vec<_>>(), [4, 1, 2, 3]);
        assert_eq!(s.pass, (4, 0));
        // A moved shortcut is read again at its new folder.
        s.want([(2, Path::new("D:\\moved"))]);
        assert!(same_path(&s.queue.iter().find(|(id, _)| *id == 2).unwrap().1, Path::new("D:\\moved")));
        s.close();
        assert!(!s.checking() && s.queued.is_empty());
        s.want(rows.iter().map(|(id, p)| (*id, p.as_path())));
        assert_eq!(s.queue.len(), 3, "the next visit reads again");
    }

    // Pull all leaves anything but a clean branch behind its upstream as it was and says
    // why; a sign-in request fails that one instead of asking.
    #[test]
    fn pull_and_fetch_outcomes_name_what_happened() {
        let refused = |failure| pull_outcome(Err(Error::Pull(failure)), Some(3));
        assert_eq!(refused(PullFailure::DirtyWorktree), Outcome::Skipped("3 changes uncommitted".into()));
        assert_eq!(pull_outcome(Err(Error::Pull(PullFailure::DirtyWorktree)), None), Outcome::Skipped("Uncommitted changes".into()));
        assert_eq!(refused(PullFailure::MissingUpstream), Outcome::Skipped("No upstream".into()));
        assert_eq!(refused(PullFailure::DetachedHead), Outcome::Skipped("No branch checked out".into()));
        assert_eq!(refused(PullFailure::OperationInProgress), Outcome::Skipped("A merge or rebase is in progress".into()));
        assert!(matches!(refused(PullFailure::StateChanged), Outcome::Failed(_)));
        assert_eq!(pull_outcome(Err(Error::Cancelled), None), Outcome::Stopped);
        let sign_in = Error::Git {
            operation: "fetch",
            code: Some(128),
            stdout: vec![],
            stderr: b"fatal: could not read Username for 'https://example.invalid': terminal prompts disabled\n".to_vec(),
        };
        assert_eq!(fetch_reason(&sign_in), "Needs a sign-in");
        assert_eq!(fetch_outcome(Ok(QuietFetchReport::default())), Outcome::Skipped("No remotes".into()));
        let report = QuietFetchReport { results: vec![(b"origin".to_vec(), Err(sign_in))], skipped: vec![] };
        assert_eq!(fetch_outcome(Ok(report)), Outcome::Failed("origin: Needs a sign-in".into()));
        assert_eq!(fetch_outcome(Err(Error::Cancelled)), Outcome::Stopped);
    }

    // The strip and the card count what happened; failures and skips need someone.
    #[test]
    fn results_summarise_and_list_who_needs_you() {
        let result = |kind, outcomes: Vec<Outcome>, stopped| BatchResult {
            kind,
            selection: Selection::All,
            scope: "Acme".into(),
            total: outcomes.len(),
            outcomes: outcomes.into_iter().enumerate().map(|(i, o)| (i as u64 + 1, o)).collect(),
            took: Duration::from_secs(3),
            stopped,
        };
        let pulled = Outcome::Pulled { commits: Some(4), head: "9f3c2e1".into() };
        let r = result(
            BatchKind::Pull,
            vec![pulled.clone(), pulled.clone(), Outcome::Skipped("Diverged".into()), Outcome::Skipped("No upstream".into()), Outcome::Skipped("Folder missing".into()), Outcome::Failed("origin: Needs a sign-in".into()), Outcome::UpToDate, Outcome::UpToDate],
            false,
        );
        assert_eq!(r.summary(), ("Pulled 2 · 3 skipped · 1 failed".into(), "2 pulled · 3 skipped · 1 failed · 2 nothing to pull".into(), Tone::Warn));
        assert_eq!(r.need_you(), [3, 4, 5, 6]);
        assert_eq!(pulled.detail(), "4 commits · now at 9f3c2e1");
        let r = result(BatchKind::Pull, vec![pulled.clone(), Outcome::UpToDate], false);
        assert_eq!(r.summary().0, "Pulled 1 · 1 nothing to pull");
        assert_eq!(r.summary().2, Tone::Good);
        let r = result(BatchKind::Fetch, vec![Outcome::Fetched { changed: true }, Outcome::Fetched { changed: false }, Outcome::Failed("x".into())], false);
        assert_eq!(r.summary().0, "Fetched 2 · 1 failed");
        let r = result(BatchKind::Fetch, vec![Outcome::Fetched { changed: true }, Outcome::Stopped, Outcome::Stopped], true);
        assert_eq!((r.summary().0, r.summary().2), ("Stopped · 1 of 3 fetched".into(), Tone::Mute));
    }
}
