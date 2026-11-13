//! Read-only projection of the existing bounded diagnostic log, never raw Git output.
use crate::{
    app::UiControl,
    operation_names::{is_read, recorded, spelled},
    style::*,
};
use eframe::egui::*;
use rocket_git::{CancellationToken, Error, ReadTask, Result};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs::{self, OpenOptions},
    io::{BufRead, BufReader, Read},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

const FILE_LIMIT: u64 = 1024 * 1024;
const RECORD_LIMIT: usize = 30_000;
const LINE_LIMIT: u64 = 16_384;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Filter {
    #[default]
    All,
    Attention,
    Cancelled,
    Succeeded,
}
impl Filter {
    pub fn label(self) -> &'static str {
        crate::i18n::tr(self.code())
    }
    fn code(self) -> &'static str {
        match self {
            Self::All => crate::i18n::mark("All"),
            Self::Attention => crate::i18n::mark("Needs attention"),
            Self::Cancelled => crate::i18n::mark("Cancelled"),
            Self::Succeeded => crate::i18n::mark("Succeeded"),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Succeeded,
    Attention,
    Cancelled,
    Incomplete,
}
impl Status {
    pub fn label(self) -> &'static str {
        crate::i18n::tr(self.code())
    }
    fn code(self) -> &'static str {
        match self {
            Self::Succeeded => crate::i18n::mark("Succeeded"),
            Self::Attention => crate::i18n::mark("Needs attention"),
            Self::Cancelled => crate::i18n::mark("Cancelled"),
            Self::Incomplete => crate::i18n::mark("Incomplete"),
        }
    }
    fn color(self) -> Color32 {
        match self {
            Self::Succeeded => GREEN(),
            Self::Attention | Self::Incomplete => AMBER(),
            Self::Cancelled => MUTED(),
        }
    }
}
#[derive(Clone, Debug)]
pub struct Entry {
    pub session: String,
    pub request: u64,
    pub operation: String,
    /// The operation in words.
    pub name: String,
    /// It only read: a known read, and no Git command wrote.
    pub read: bool,
    pub time: u64,
    pub duration: Option<u64>,
    pub status: Status,
    pub records: Vec<Value>,
    pub incomplete: bool,
    search: String,
    problem: Option<Problem>,
}
impl Entry {
    pub fn key(&self) -> String {
        format!("{}:{}", self.session, self.request)
    }
    /// The row says why it needs a look under its name.
    fn tall(&self) -> bool {
        matches!(self.status, Status::Attention | Status::Incomplete)
    }
    pub fn summary(&self) -> String {
        serde_json::to_string_pretty(&json!({"schema":1,"session":self.session,"request":self.request,"operation":self.operation,"time_unix_ms":self.time,"elapsed_ms":self.duration,"status":self.status.code(),"incomplete":self.incomplete,"records":self.records.iter().take(200).collect::<Vec<_>>(),"records_omitted":self.records.len().saturating_sub(200),"scope":"Structured diagnostic fields only; repository names, arguments and raw output are excluded."})).expect("diagnostic values serialize")
    }
}
#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub entries: Vec<Entry>,
    pub records: usize,
    pub sessions: usize,
    pub gaps: usize,
    pub partial: usize,
    pub dropped: u64,
    pub elapsed_ms: u64,
}

fn invalid(message: &str) -> Error {
    Error::InvalidInput(message.into())
}
fn redirected(m: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        m.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        m.file_type().is_symlink()
    }
}
fn label<'a>(v: &'a Value, key: &str) -> &'a str {
    v[key].as_str().unwrap_or("")
}
fn number(v: &Value, key: &str) -> u64 {
    v[key].as_u64().unwrap_or(0)
}
fn identifier(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 96
        && s.bytes().all(|b| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-' || b == b'.'
        })
}
fn session_id(s: &str) -> bool {
    let parts: Vec<_> = s.split('-').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.len() <= 32 && p.bytes().all(|b| b.is_ascii_hexdigit()))
}
fn project(v: Value) -> Result<Value> {
    if v["schema"] != 1
        || !v["seq"].as_u64().is_some_and(|n| n > 0)
        || v["request"].as_u64().is_none()
        || !v["session"].as_str().is_some_and(session_id)
    {
        return Err(invalid(
            "A record in the log is damaged.",
        ));
    }
    if !v["event"].as_str().is_some_and(identifier)
        || (matches!(label(&v, "event"), "request_end" | "mcp_request")
            && !v["outcome"].as_str().is_some_and(identifier))
    {
        return Err(invalid("A record in the log is damaged."));
    }
    let mut safe = serde_json::Map::new();
    safe.insert("session".into(), v["session"].clone());
    for key in [
        "schema",
        "seq",
        "request",
        "time_unix_ms",
        "elapsed_ms",
        "native_request",
        "queue_ms",
        "validation_ms",
        "phase_ms",
        "cancel_to_stop_us",
        "prepare_us",
        "verify_us",
        "spawn_us",
        "contain_us",
        "execute_us",
        "drain_us",
        "id",
        "dropped_before",
        "objects",
        "object_bytes",
        "checked",
        "total",
        "conflict_paths",
        "branches",
    ] {
        if let Some(n) = v[key].as_u64() {
            safe.insert(key.into(), json!(n));
        }
    }
    if let Some(n) = v["exit_code"].as_i64() {
        safe.insert("exit_code".into(), json!(n));
    }
    for key in [
        "event",
        "operation",
        "step",
        "outcome",
        "failure_code",
        "level",
        "remote_outcome",
        "creation",
        "verification",
        "fetch_outcome",
        "update_outcome",
        "operation_state",
        "version",
        "clone_command",
        "publication",
        "reopen",
        "cleanup",
    ] {
        if let Some(s) = v[key].as_str() {
            safe.insert(
                key.into(),
                json!(if identifier(s) { s } else { "unrecognized" }),
            );
        }
    }
    for (key, allowed) in [
        (
            "reason",
            &[
                "none",
                "merge_settings",
                "merge_attributes",
                "attributes_changing",
                "submodules",
                "replaced_objects",
                "prediction_limit",
                "prediction_failed",
                "prediction_timeout",
            ][..],
        ),
        ("kind", &["read", "write"][..]),
        (
            "tool",
            &[
                "rocket_open",
                "rocket_close",
                "rocket_state",
                "rocket_click",
                "rocket_drag",
                "rocket_type",
                "rocket_scroll",
                "rocket_resize",
                "rocket_capture",
                "rocket_logs",
                "rocket_wait",
                "rocket_escape",
                "rocket_cancel_input",
                "rocket_terminal_enter",
                "rocket_terminal_probe",
                "other",
            ][..],
        ),
        (
            "phase",
            &[
                "queued",
                "ui",
                "guard",
                "validation",
                "input",
                "delivered",
                "other",
            ][..],
        ),
    ] {
        if let Some(value) = v[key].as_str() {
            safe.insert(
                key.into(),
                json!(if allowed.contains(&value) {
                    value
                } else {
                    "unrecognized"
                }),
            );
        }
    }
    for key in [
        crate::i18n::tr("complete"),
        "input_delivered",
        "verified",
        "command_succeeded",
        "error_diagnostics",
        "local_verified",
        "refreshed",
        "remote_added",
        "has_errors",
    ] {
        if let Some(b) = v[key].as_bool() {
            safe.insert(key.into(), json!(b));
        }
    }
    Ok(Value::Object(safe))
}
pub fn read(directory: &Path, cancel: &CancellationToken) -> Result<Snapshot> {
    let began = Instant::now();
    let mut records = BTreeMap::new();
    let mut partial = 0;
    cancel.check()?;
    let root = directory
        .symlink_metadata()
        .map_err(|_| invalid("Diagnostics directory is unavailable. Refresh to retry."))?;
    if !root.is_dir() || redirected(&root) {
        return Err(invalid(
            "Diagnostics directory is redirected; read refused.",
        ));
    }
    for path in rocket_git::DiagnosticLog::retained_files(directory)
        .map_err(|_| invalid("Cannot read diagnostic retention paths."))? {
        cancel.check()?;
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.share_mode(7).custom_flags(0x0020_0000);
        }
        let file = match options.open(&path) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => {
                return Err(invalid(
                    "Cannot read diagnostics; the log may be rotating. Refresh to retry.",
                ));
            }
        };
        let meta = file
            .metadata()
            .map_err(|_| invalid("Cannot inspect diagnostic file."))?;
        let maximum = if path.parent() == Some(directory) { FILE_LIMIT } else { LINE_LIMIT };
        if !meta.is_file() || redirected(&meta) || meta.len() > maximum {
            return Err(invalid(
                "Diagnostic file is redirected or exceeds its retention bound.",
            ));
        }
        // Snapshot the initial length, allowing concurrent appends/rotation without following them.
        let mut reader = BufReader::new(file.take(meta.len()));
        loop {
            cancel.check()?;
            let mut line = Vec::new();
            let n = (&mut reader)
                .take(LINE_LIMIT + 1)
                .read_until(b'\n', &mut line)
                .map_err(|_| invalid("Diagnostic read failed. Refresh to retry."))?;
            if n == 0 {
                break;
            }
            if n as u64 > LINE_LIMIT {
                return Err(invalid("Diagnostic record exceeds the 16 KiB bound."));
            }
            if !line.ends_with(b"\n") {
                partial += 1;
                break;
            }
            let v = project(serde_json::from_slice(&line).map_err(|_| {
                invalid("A record in the log is damaged.")
            })?)?;
            let key = (label(&v, "session").to_owned(), number(&v, "seq"));
            if let Some(previous) = records.insert(key, v.clone())
                && previous != v
            {
                return Err(invalid(
                    "Conflicting diagnostic sequences; refresh after rotation settles.",
                ));
            }
            if records.len() > RECORD_LIMIT {
                return Err(invalid("Diagnostic record count exceeds its bound."));
            }
        }
    }
    cancel.check()?;
    let mut result = aggregate(records.into_values().collect(), cancel)?;
    result.partial = partial;
    result.elapsed_ms = began.elapsed().as_millis() as u64;
    Ok(result)
}
fn aggregate(records: Vec<Value>, cancel: &CancellationToken) -> Result<Snapshot> {
    let mut result = Snapshot {
        records: records.len(),
        ..Default::default()
    };
    let mut by_session: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    for r in records {
        by_session
            .entry(label(&r, "session").into())
            .or_default()
            .push(r);
    }
    result.sessions = by_session.len();
    for (session, mut records) in by_session {
        records.sort_by_key(|r| number(r, "seq"));
        let mut gaps = vec![];
        let mut previous = 0;
        let mut dropped = 0;
        for r in &records {
            let seq = number(r, "seq");
            if seq > previous + 1 {
                gaps.push((previous, seq));
            }
            previous = seq;
            dropped = dropped.max(number(r, "dropped_before"));
        }
        result.gaps += gaps.len();
        result.dropped = result.dropped.saturating_add(dropped);
        let mut requests: BTreeMap<u64, Vec<Value>> = BTreeMap::new();
        for r in records {
            let request = number(&r, "request");
            if request != 0 {
                requests.entry(request).or_default().push(r);
            }
        }
        for (request, events) in requests {
            cancel.check()?;
            let single = (events.len() == 1 && label(&events[0], "event") == "mcp_request")
                .then_some(&events[0]);
            let start = events
                .iter()
                .find(|r| label(r, "event") == "request_start")
                .or(single);
            let end = events
                .iter()
                .rev()
                .find(|r| label(r, "event") == "request_end")
                .or(single);
            let min = number(events.first().unwrap(), "seq");
            let max = number(events.last().unwrap(), "seq");
            let incomplete = start.is_none()
                || end.is_none()
                || gaps.iter().any(|(a, b)| *a >= min && *b <= max)
                || events.iter().any(|r| number(r, "dropped_before") > 0);
            let cancelled = events.iter().any(|r| label(r, "outcome") == "cancelled");
            let failed = events.iter().any(|r| {
                failed_result(r)
                    || (label(r, "level") == "warn" && label(r, "event") != "git_end")
                    || r[crate::i18n::tr("complete")] == false
                    || r["verified"] == false
                    || r["error_diagnostics"] == true
                    || matches!(
                        label(r, "remote_outcome"),
                        "unknown" | "rejected" | "not_started"
                    )
            });
            let status = if cancelled {
                Status::Cancelled
            } else if failed {
                Status::Attention
            } else if incomplete {
                Status::Incomplete
            } else {
                Status::Succeeded
            };
            let main = start.or(end).unwrap_or(&events[0]);
            let operation = label(main, "operation").to_owned();
            let time = number(main, "time_unix_ms");
            let duration = end.and_then(|r| r["elapsed_ms"].as_u64());
            let name = recorded(&operation);
            let read = is_read(&operation)
                && !events
                    .iter()
                    .any(|r| label(r, "event") == "git_start" && label(r, "kind") == "write");
            let problem = problem(status, &events);
            let mut search = events
                .iter()
                .flat_map(|r| {
                    [
                        label(r, "operation"),
                        label(r, "step"),
                        label(r, "failure_code"),
                        label(r, "outcome"),
                        label(r, "tool"),
                        label(r, "phase"),
                    ]
                })
                .collect::<Vec<_>>()
                .join(" ");
            // What the row shows is found too, not only the raw names.
            search.push(' ');
            search.push_str(&name);
            if let Some(problem) = &problem {
                search.push(' ');
                search.push_str(&problem.headline);
            }
            let search = search.to_lowercase();
            result.entries.push(Entry {
                session: session.clone(),
                request,
                operation,
                name,
                read,
                time,
                duration,
                status,
                records: events,
                incomplete,
                search,
                problem,
            });
        }
    }
    result
        .entries
        .sort_by(|a, b| (b.time, &b.session, b.request).cmp(&(a.time, &a.session, a.request)));
    Ok(result)
}
pub fn timestamp(ms: u64) -> String {
    // Bounded to four-digit years for display.
    if ms > 253_402_300_799_999 {
        return crate::i18n::tr("Time unavailable").into();
    }
    let t = civil(ms);
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02} UTC",
        t.year, t.month, t.day, t.hour, t.minute, t.second
    )
}
/// The Gregorian civil date and time in UTC, from epoch days.
fn civil(ms: u64) -> rocket_git::LocalTime {
    let z = (ms / 86_400_000) as i64 + 719468;
    let era = z / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let mut y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    y += i64::from(m <= 2);
    let s = ms / 1000;
    rocket_git::LocalTime {
        year: y as u16,
        month: m as u8,
        day: d as u8,
        hour: (s / 3600 % 24) as u8,
        minute: (s / 60 % 60) as u8,
        second: (s % 60) as u8,
    }
}

pub struct Panel {
    pub open: bool,
    pub query: String,
    pub filter: Filter,
    pub current_only: bool,
    pub automatic: bool,
    /// Also list reads that went fine. Anything else always shows.
    pub show_reads: bool,
    pub snapshot: Snapshot,
    pub selected: Option<String>,
    pub error: Option<String>,
    pub notice: Option<String>,
    task: Option<ReadTask<Snapshot>>,
    /// When the running read began; a quick one changes nothing on screen.
    started: Option<Instant>,
    last_read: Option<Instant>,
    /// A read has finished, so an empty list is really empty.
    read_once: bool,
    dirty: bool,
    /// Copy summary was clicked for this operation, for a short "Copied".
    copied: Option<(String, Instant)>,
    /// Sessions folded in the list.
    folded: HashSet<String>,
    /// The selected operation shows every record instead of its steps.
    records: bool,
    /// The selected operation's setup steps are unfolded.
    setup_open: bool,
    /// A narrow window shows Details instead of Operations.
    details_tab: bool,
    /// The selected operation's steps: its key, record count and rows.
    steps: Option<(String, usize, Vec<StepRow>)>,
}
impl Default for Panel {
    fn default() -> Self {
        Self {
            open: false,
            query: String::new(),
            filter: Filter::All,
            current_only: false,
            automatic: true,
            show_reads: false,
            snapshot: Snapshot::default(),
            selected: None,
            error: None,
            notice: None,
            task: None,
            started: None,
            last_read: None,
            read_once: false,
            dirty: true,
            copied: None,
            folded: HashSet::new(),
            records: false,
            setup_open: false,
            details_tab: false,
            steps: None,
        }
    }
}
impl Panel {
    pub fn loading(&self) -> bool {
        self.task.is_some()
    }
    /// A read has run long enough to offer Stop.
    fn slow(&self) -> bool {
        self.started.is_some_and(|t| t.elapsed() >= SLOW_READ)
    }
    pub fn show(&mut self) {
        self.open = true;
        self.dirty = true;
    }
    pub fn close(&mut self) {
        self.open = false;
        self.task = None;
        self.started = None;
    }
    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }
    pub fn refresh(&mut self) {
        self.dirty = true;
    }
    pub fn cancel(&mut self) {
        self.task = None;
        self.started = None;
        self.last_read = Some(Instant::now());
        self.dirty = false;
        self.automatic = false;
        self.notice = Some(crate::i18n::tr("Read stopped. Live updates are paused.").into());
    }
    pub fn poll(&mut self, directory: PathBuf, ctx: &Context) {
        if !self.open {
            self.task = None;
            self.started = None;
            return;
        }
        if let Some(result) = self.task.as_mut().and_then(ReadTask::try_finish) {
            self.task = None;
            self.started = None;
            self.last_read = Some(Instant::now());
            match result {
                Ok(s) => {
                    self.snapshot = s;
                    self.error = None;
                    self.notice = None;
                    self.read_once = true;
                }
                Err(Error::InvalidInput(message)) => self.error = Some(message),
                Err(e) => self.error = Some(e.to_string()),
            }
            ctx.request_repaint();
        }
        if self.task.is_none()
            && (self.dirty
                || (self.automatic
                    && self
                        .last_read
                        .is_none_or(|t| t.elapsed() > Duration::from_secs(5))))
        {
            self.dirty = false;
            self.last_read = Some(Instant::now());
            match ReadTask::spawn(move |c| read(&directory, &c)) {
                Ok(task) => {
                    self.task = Some(task);
                    self.started = Some(Instant::now());
                }
                Err(e) => self.error = Some(e.to_string()),
            }
        }
        if self.loading() || self.automatic {
            ctx.request_repaint_after(Duration::from_millis(if self.loading() { 50 } else { 500 }));
        }
    }
    /// Whether `e` is left out for being a read that went fine. A search looks at everything.
    fn hidden(&self, e: &Entry) -> bool {
        !self.show_reads && self.query.trim().is_empty() && e.read && e.status == Status::Succeeded
    }
    /// Every entry within the search and filters; automation sees reads either way.
    pub fn matching(&self, current: &str) -> Vec<usize> {
        let q = self.query.trim().to_lowercase();
        self.snapshot
            .entries
            .iter()
            .enumerate()
            .filter(|(_, e)| {
                (!self.current_only || e.session == current)
                    && (q.is_empty() || e.search.contains(&q))
                    && match self.filter {
                        Filter::All => true,
                        Filter::Attention => {
                            matches!(e.status, Status::Attention | Status::Incomplete)
                        }
                        Filter::Cancelled => e.status == Status::Cancelled,
                        Filter::Succeeded => e.status == Status::Succeeded,
                    }
            })
            .map(|(i, _)| i)
            .collect()
    }
    /// The rows the list shows.
    fn visible(&self, current: &str) -> Vec<usize> {
        let mut rows = self.matching(current);
        rows.retain(|i| !self.hidden(&self.snapshot.entries[*i]));
        rows
    }
    pub fn state(&self, current: &str) -> Value {
        let matching = self.matching(current);
        json!({"open":self.open,"loading":self.loading(),"query":self.query,"filter":self.filter.code(),"current_only":self.current_only,"automatic":self.automatic,"show_reads":self.show_reads,"records_view":self.records,"error":self.error,"notice":self.notice,"records":self.snapshot.records,"sessions":self.snapshot.sessions,"gaps":self.snapshot.gaps,"partial":self.snapshot.partial,"dropped":self.snapshot.dropped,"read_ms":self.snapshot.elapsed_ms,"matched":matching.len(),"rows":matching.into_iter().take(200).map(|i|{let e=&self.snapshot.entries[i];json!({"index":i,"key":e.key(),"operation":e.operation,"status":e.status.code(),"session":e.session,"request":e.request,"elapsed_ms":e.duration,"incomplete":e.incomplete})}).collect::<Vec<_>>(),"selected":self.selected.as_ref().and_then(|k|self.snapshot.entries.iter().find(|e|e.key()==*k)).map(|e|serde_json::from_str::<Value>(&e.summary()).expect("own summary"))})
    }
    /// How many entries each status filter shows, within the search and session scope.
    fn counts(&self, current: &str) -> [usize; 4] {
        let q = self.query.trim().to_lowercase();
        let mut counts = [0; 4];
        for e in &self.snapshot.entries {
            if (self.current_only && e.session != current)
                || (!q.is_empty() && !e.search.contains(&q))
                || self.hidden(e)
            {
                continue;
            }
            counts[0] += 1;
            match e.status {
                Status::Attention | Status::Incomplete => counts[1] += 1,
                Status::Cancelled => counts[2] += 1,
                Status::Succeeded => counts[3] += 1,
            }
        }
        counts
    }
    /// How many reads that went fine are in the session scope, for Show reads.
    fn reads(&self, current: &str) -> usize {
        self.snapshot
            .entries
            .iter()
            .filter(|e| e.read && e.status == Status::Succeeded)
            .filter(|e| !self.current_only || e.session == current)
            .count()
    }

    pub fn ui(&mut self, ui: &mut Ui, widgets: &mut HashMap<String, UiControl>, current: &str) {
        let matches = self.visible(current);
        if self.selected.as_ref().is_none_or(|k| {
            !matches
                .iter()
                .any(|i| self.snapshot.entries[*i].key() == *k)
        }) {
            self.selected = matches.first().map(|i| self.snapshot.entries[*i].key());
        }
        ui.spacing_mut().item_spacing.y = 0.0;
        // The body stays inside the page even if a header row overflows a narrow window.
        let page = ui.max_rect();
        let narrow = page.width() < NARROW;
        self.header(ui, widgets, narrow);
        self.filters(ui, widgets, current, narrow);
        // Nothing read yet: the card itself says why, so the strip would only repeat it.
        let unread = !self.read_once && self.snapshot.entries.is_empty();
        if !unread {
            self.strip(ui, widgets, narrow);
        }
        let inset = if narrow { 20.0 } else { 24.0 };
        let body = Rect::from_min_max(
            pos2(page.left() + inset, ui.cursor().top()),
            pos2(page.right() - inset, page.bottom() - 16.0),
        );
        ui.allocate_rect(body, Sense::hover());
        if unread {
            self.unread_card(ui, widgets, body);
        } else if matches.is_empty() {
            self.empty_card(ui, widgets, body, current);
        } else if narrow {
            crate::pages::card_back(ui, body);
            ui.scope_builder(UiBuilder::new().max_rect(body), |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                crate::pages::card_bar(ui, |ui| self.pane_tabs(ui, widgets, current, matches.len()));
                let rest = Rect::from_min_max(ui.cursor().min, body.max);
                if self.details_tab {
                    self.details(ui, widgets, rest, false);
                } else {
                    self.list(ui, widgets, rest, &matches, current, true);
                }
            });
        } else {
            let width = (body.width() * 0.42).clamp(320.0, 470.0);
            let left = Rect::from_min_size(body.min, vec2(width, body.height()));
            let right = Rect::from_min_max(pos2(left.right() + 16.0, body.top()), body.max);
            crate::pages::card_back(ui, left);
            ui.scope_builder(UiBuilder::new().max_rect(left), |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                crate::pages::card_bar(ui, |ui| {
                    ui.label(RichText::new(crate::i18n::tr("Operations")).font(semibold(13.0)));
                    ui.label(RichText::new(crate::pages::number_text(matches.len())).size(12.0).color(HINT()));
                });
                let rest = Rect::from_min_max(ui.cursor().min, left.max);
                self.list(ui, widgets, rest, &matches, current, false);
            });
            crate::pages::card_back(ui, right);
            ui.scope_builder(UiBuilder::new().max_rect(right), |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                self.details(ui, widgets, right, true);
            });
        }
    }

    fn header(&mut self, ui: &mut Ui, widgets: &mut HashMap<String, UiControl>, narrow: bool) {
        let mut actions = |ui: &mut Ui, this: &mut Self| {
            let close = crate::pages::ghost(ui, true, crate::i18n::tr("Close"));
            widgets.insert("activity-close".into(), (&close).into());
            if close.clicked() {
                this.close();
            }
            // Refresh and Stop share one place, so nothing beside them moves.
            let first = !this.read_once && this.snapshot.entries.is_empty();
            let width = if narrow { 30.0 } else { 92.0 };
            if this.slow() && !first {
                let stop = crate::pages::slot_button(ui, None, crate::i18n::tr("Stop"), width, true);
                widgets.insert("activity-cancel".into(), (&stop).into());
                if stop.clicked() {
                    this.cancel();
                }
            } else {
                let refresh = crate::pages::slot_button(ui, Some(Icon::Refresh), crate::i18n::tr("Refresh"), width, !first);
                widgets.insert("activity-refresh".into(), (&refresh).into());
                if refresh.clicked() {
                    this.refresh();
                }
            }
            ui.add_space(4.0);
            ui.label(RichText::new(crate::i18n::tr("Live")).color(MUTED()));
            let live = switch(ui, &mut this.automatic, crate::i18n::tr("Live updates"))
                .on_hover_text(crate::i18n::tr("Read new records every few seconds"));
            widgets.insert("activity-auto".into(), (&live).into());
            if !this.automatic && !narrow
                && let Some(read) = this.last_read
            {
                let seconds = read.elapsed().as_secs();
                let when = ago(seconds, (unix_ms() / 1000) as i64 - seconds as i64);
                ui.label(RichText::new(crate::i18n::trf!("Read {when}", when)).size(12.0).color(HINT()));
            }
        };
        if narrow {
            Frame::new()
                .inner_margin(Margin {
                    left: 20,
                    right: 20,
                    top: 10,
                    bottom: 8,
                })
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.spacing_mut().item_spacing.x = 8.0;
                            actions(ui, self);
                            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                                ui.add(
                                    Label::new(RichText::new(crate::i18n::tr("Activity & diagnostics")).font(semibold(16.0)))
                                        .truncate(),
                                );
                            });
                        });
                    });
                });
        } else {
            crate::pages::header(
                ui,
                Icon::Activity,
                crate::i18n::tr("Activity & diagnostics"),
                crate::i18n::tr("What Rocket recorded on this computer, newest first."),
                |ui| actions(ui, self),
            );
        }
    }

    fn filters(
        &mut self,
        ui: &mut Ui,
        widgets: &mut HashMap<String, UiControl>,
        current: &str,
        narrow: bool,
    ) {
        let counts = self.counts(current);
        let reads = self.reads(current);
        let inset = if narrow { 20 } else { 24 };
        Frame::new()
            .inner_margin(Margin {
                left: inset,
                right: inset,
                top: 0,
                bottom: 12,
            })
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
                    // What the segments and buttons beside it need.
                    let beside = if narrow { 448.0 } else { 720.0 };
                    let width = (ui.available_width() - beside).clamp(150.0, 280.0);
                    let hint = if narrow { crate::i18n::tr("Search") } else { crate::i18n::tr("Search operations or error codes") };
                    let r = crate::pages::search_box(ui, &mut self.query, "activity-search", hint, width);
                    widgets.insert("activity-search".into(), (&r).into());
                    let filters = [Filter::All, Filter::Attention, Filter::Cancelled, Filter::Succeeded];
                    let items: Vec<(&str, usize, bool)> = filters
                        .iter()
                        .zip(counts)
                        .map(|(f, n)| (f.label(), n, self.filter == *f))
                        .collect();
                    for (r, (f, id)) in crate::pages::count_segments(ui, &items).into_iter().zip([
                        (Filter::All, "all"),
                        (Filter::Attention, "errors"),
                        (Filter::Cancelled, "cancelled"),
                        (Filter::Succeeded, "succeeded"),
                    ]) {
                        widgets.insert(format!("activity-{id}"), (&r).into());
                        if r.clicked() {
                            self.filter = f;
                        }
                    }
                    if !narrow {
                        self.chips(ui, widgets, reads, crate::i18n::tr("This session only"), CONTROL);
                    }
                });
            });
    }

    /// This session only and Show reads.
    fn chips(
        &mut self,
        ui: &mut Ui,
        widgets: &mut HashMap<String, UiControl>,
        reads: usize,
        session: &str,
        height: f32,
    ) {
        let r = chip(ui, session, None, self.current_only, height)
            .on_hover_text(crate::i18n::tr("Only operations from this run of Rocket"));
        widgets.insert("activity-current".into(), (&r).into());
        if r.clicked() {
            self.current_only = !self.current_only;
        }
        let r = chip(ui, crate::i18n::tr("Show reads"), Some(reads), self.show_reads, height)
            .on_hover_text(crate::i18n::tr("Also list the reads Rocket does by itself"));
        widgets.insert("activity-reads".into(), (&r).into());
        if r.clicked() {
            self.show_reads = !self.show_reads;
        }
    }

    /// A read that failed or was stopped, above the list that stays.
    fn strip(&mut self, ui: &mut Ui, widgets: &mut HashMap<String, UiControl>, narrow: bool) {
        if self.error.is_none() && self.notice.is_none() {
            return;
        }
        let inset = if narrow { 20 } else { 24 };
        Frame::new()
            .inner_margin(Margin {
                left: inset,
                right: inset,
                top: 0,
                bottom: 10,
            })
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 6.0;
                if let Some(error) = self.error.clone() {
                    let text = crate::i18n::trf!("Couldn't read the latest records. {error} This is the last good read.", error);
                    if let Some(r) = crate::pages::notice_strip(ui, RED(), &text, Some(crate::i18n::tr("Try again"))) {
                        widgets.insert("activity-retry".into(), (&r).into());
                        if r.clicked() {
                            self.refresh();
                        }
                    }
                } else if let Some(notice) = self.notice.clone()
                    && let Some(r) = crate::pages::notice_strip(ui, AMBER(), &notice, Some(crate::i18n::tr("Resume")))
                {
                    widgets.insert("activity-resume".into(), (&r).into());
                    if r.clicked() {
                        self.notice = None;
                        self.automatic = true;
                        self.refresh();
                    }
                }
            });
    }

    /// Nothing has been read: reading, stopped or failed, in one card.
    fn unread_card(&mut self, ui: &mut Ui, widgets: &mut HashMap<String, UiControl>, rect: Rect) {
        crate::pages::card_back(ui, rect);
        ui.scope_builder(UiBuilder::new().max_rect(rect), |ui| {
            if let Some(error) = self.error.clone() {
                centred(ui, rect, 120.0, |ui| {
                    let (mark, _) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::hover());
                    icon(ui.painter(), mark, Icon::Warning, AMBER());
                    ui.add_space(8.0);
                    ui.label(RichText::new(crate::i18n::tr("Couldn't read the log")).font(semibold(14.0)));
                    ui.add_space(4.0);
                    ui.label(RichText::new(error).color(MUTED()));
                    ui.add_space(10.0);
                    let r = crate::pages::primary(ui, true, crate::i18n::tr("Try again"));
                    widgets.insert("activity-retry".into(), (&r).into());
                    if r.clicked() {
                        self.refresh();
                    }
                });
            } else if let Some(notice) = self.notice.clone() {
                centred(ui, rect, 80.0, |ui| {
                    ui.label(RichText::new(notice).color(MUTED()));
                    ui.add_space(10.0);
                    let r = crate::pages::primary(ui, true, crate::i18n::tr("Resume"));
                    widgets.insert("activity-resume".into(), (&r).into());
                    if r.clicked() {
                        self.notice = None;
                        self.automatic = true;
                        self.refresh();
                    }
                });
            } else {
                crate::pages::loading_with(ui, crate::i18n::tr("Reading the log"), 40.0, |ui| {
                    ui.add_space(8.0);
                    let r = ui.add_enabled(self.loading(), Button::new(crate::i18n::tr("Stop")));
                    widgets.insert("activity-cancel".into(), (&r).into());
                    if r.clicked() {
                        self.cancel();
                    }
                });
            }
        });
    }

    /// The list is empty: nothing recorded, only reads, or nothing that matches.
    fn empty_card(
        &mut self,
        ui: &mut Ui,
        widgets: &mut HashMap<String, UiControl>,
        rect: Rect,
        current: &str,
    ) {
        crate::pages::card_back(ui, rect);
        ui.scope_builder(UiBuilder::new().max_rect(rect), |ui| {
            if self.snapshot.entries.is_empty() {
                centred(ui, rect, 44.0, |ui| {
                    ui.label(RichText::new(crate::i18n::tr("Nothing recorded yet")).font(semibold(14.0)));
                    ui.add_space(4.0);
                    ui.label(RichText::new(crate::i18n::tr("Operations show here as they run.")).color(MUTED()));
                });
                return;
            }
            // Rows exist, but every one is a read that went fine.
            if !self.matching(current).is_empty() {
                centred(ui, rect, 88.0, |ui| {
                    ui.label(RichText::new(crate::i18n::tr("Only reads so far")).font(semibold(14.0)));
                    ui.add_space(4.0);
                    ui.label(RichText::new(crate::i18n::tr("Nothing else matches these filters.")).color(MUTED()));
                    ui.add_space(10.0);
                    let r = ui.add(Button::new(crate::i18n::tr("Show reads")));
                    widgets.insert("activity-show-reads".into(), (&r).into());
                    if r.clicked() {
                        self.show_reads = true;
                    }
                });
                return;
            }
            centred(ui, rect, 88.0, |ui| {
                ui.label(RichText::new(crate::i18n::tr("No operations match")).font(semibold(14.0)));
                ui.add_space(4.0);
                let query = self.query.trim();
                let text = if query.is_empty() {
                    crate::i18n::tr("Nothing with these filters.").to_owned()
                } else {
                    crate::i18n::trf!("Nothing for “{query}” with these filters.", query)
                };
                ui.label(RichText::new(text).color(MUTED()));
                ui.add_space(10.0);
                let r = ui.add(Button::new(crate::i18n::tr("Clear filters")));
                widgets.insert("activity-clear".into(), (&r).into());
                if r.clicked() {
                    self.query.clear();
                    self.filter = Filter::All;
                    self.current_only = false;
                }
            });
        });
    }

    /// A narrow window's Operations and Details tabs, with what sits beside the filters
    /// in a wide one.
    fn pane_tabs(
        &mut self,
        ui: &mut Ui,
        widgets: &mut HashMap<String, UiControl>,
        current: &str,
        count: usize,
    ) {
        let items = [(crate::i18n::tr("Operations"), Some(count), !self.details_tab), (crate::i18n::tr("Details"), None, self.details_tab)];
        for (r, (id, details)) in crate::pages::tab_segments(ui, &items)
            .into_iter()
            .zip([("activity-tab-list", false), ("activity-tab-details", true)])
        {
            widgets.insert(id.into(), (&r).into());
            if r.clicked() {
                self.details_tab = details;
            }
        }
        if !self.details_tab {
            let reads = self.reads(current);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                // Added right to left, Show reads ends up last.
                let r = chip(ui, crate::i18n::tr("Show reads"), Some(reads), self.show_reads, 24.0)
                    .on_hover_text(crate::i18n::tr("Also list the reads Rocket does by itself"));
                widgets.insert("activity-reads".into(), (&r).into());
                if r.clicked() {
                    self.show_reads = !self.show_reads;
                }
                let r = chip(ui, crate::i18n::tr("This session"), None, self.current_only, 24.0)
                    .on_hover_text(crate::i18n::tr("Only operations from this run of Rocket"));
                widgets.insert("activity-current".into(), (&r).into());
                if r.clicked() {
                    self.current_only = !self.current_only;
                }
            });
        }
    }

    fn list(
        &mut self,
        ui: &mut Ui,
        widgets: &mut HashMap<String, UiControl>,
        rect: Rect,
        matches: &[usize],
        current: &str,
        narrow: bool,
    ) {
        // Group by session, this session first, keeping the newest-first order. A search
        // opens every group, so a match is never behind a fold.
        enum Item {
            Header(String, String, usize, bool, usize),
            Row(usize),
        }
        let foot = 36.0;
        let area = Rect::from_min_max(rect.min, pos2(rect.right(), rect.bottom() - foot));
        let searching = !self.query.trim().is_empty();
        let entries = &self.snapshot.entries;
        let mut sessions: Vec<&str> = Vec::new();
        for &i in matches {
            let session = entries[i].session.as_str();
            if !sessions.contains(&session) {
                sessions.push(session);
            }
        }
        sessions.sort_by_key(|s| *s != current);
        let now = unix_ms();
        let mut items = Vec::with_capacity(matches.len() + sessions.len());
        for (group, session) in sessions.into_iter().enumerate() {
            let rows: Vec<usize> = matches.iter().copied().filter(|i| entries[*i].session == session).collect();
            let label = if session == current {
                crate::i18n::tr("This session").to_owned()
            } else {
                let newest = rows.first().map_or(0, |i| entries[*i].time);
                crate::i18n::trf!("Earlier session · {day} {time}", day = day(newest, now), time = clock(newest))
            };
            let open = searching || !self.folded.contains(session);
            items.push(Item::Header(session.to_owned(), label, rows.len(), open, group));
            if open {
                items.extend(rows.into_iter().map(Item::Row));
            }
        }
        let heights: Vec<f32> = items
            .iter()
            .map(|item| match item {
                Item::Header(..) => GROUP,
                Item::Row(i) if entries[*i].tall() => TALL_ROW,
                Item::Row(_) => ROW,
            })
            .collect();
        let mut tops = Vec::with_capacity(heights.len());
        let mut total = 0.0;
        for h in &heights {
            tops.push(total);
            total += h;
        }
        let selected = self.selected.clone();
        let mut picked = None;
        let mut fold = None;
        let out = ui
            .scope_builder(UiBuilder::new().max_rect(area), |ui| {
                ScrollArea::vertical()
                    .id_salt("activity-history-list")
                    .auto_shrink([false, false])
                    .show_viewport(ui, |ui, viewport| {
                        ui.set_height(total);
                        let origin = ui.min_rect().min;
                        let width = ui.available_width();
                        let first = tops.partition_point(|t| *t + TALL_ROW < viewport.min.y);
                        for (k, item) in items.iter().enumerate().skip(first) {
                            if tops[k] > viewport.max.y {
                                break;
                            }
                            let rect = Rect::from_min_size(origin + vec2(0.0, tops[k]), vec2(width, heights[k]));
                            match item {
                                Item::Header(session, label, count, open, group) => {
                                    let response = ui.interact(rect, ui.scope_id().with(("activity-group", k)), Sense::click());
                                    paint_group(ui, rect, label, *count, *open, response.hovered());
                                    response.widget_info(|| WidgetInfo::selected(Role::Button, !searching, *open, label));
                                    let visible = rect.intersect(ui.clip_rect());
                                    if visible.is_positive() {
                                        widgets.insert(
                                            format!("activity-group-{group}"),
                                            UiControl {
                                                rect: visible,
                                                enabled: !searching,
                                            },
                                        );
                                    }
                                    if response.clicked() && !searching {
                                        fold = Some(session.clone());
                                    }
                                }
                                Item::Row(i) => {
                                    let e = &entries[*i];
                                    let response = ui.interact(rect, ui.scope_id().with(("activity-row", *i)), Sense::click());
                                    let on = selected.as_deref() == Some(e.key().as_str());
                                    paint_entry(ui, rect, e, on, response.hovered());
                                    response.widget_info(|| {
                                        WidgetInfo::selected(Role::Button, true, on, format!("{} · {}", e.name, e.status.label()))
                                    });
                                    let visible = rect.intersect(ui.clip_rect());
                                    if visible.is_positive() {
                                        widgets.insert(
                                            format!("activity-row-{i}"),
                                            UiControl {
                                                rect: visible,
                                                enabled: true,
                                            },
                                        );
                                    }
                                    if response.clicked() {
                                        picked = Some(e.key());
                                    }
                                }
                            }
                        }
                    })
            })
            .inner;
        widgets.insert(
            "activity-list".into(),
            UiControl {
                rect: out.inner_rect,
                enabled: true,
            },
        );
        if let Some(key) = picked {
            self.selected = Some(key);
            self.details_tab = narrow;
        }
        if let Some(session) = fold
            && !self.folded.remove(&session)
        {
            self.folded.insert(session);
        }
        // The log's health, in words; the numbers are on hover and in the guide.
        let bar = Rect::from_min_max(pos2(rect.left(), area.bottom()), rect.max);
        let mut foot_ui = card_foot(ui, bar);
        let s = &self.snapshot;
        let complete = s.gaps + s.partial == 0 && s.dropped == 0;
        let color = if complete { GREEN() } else { AMBER() };
        let (mark, _) = foot_ui.allocate_exact_size(vec2(13.0, 13.0), Sense::hover());
        icon(foot_ui.painter(), mark, if complete { Icon::Check } else { Icon::Warning }, color);
        let text = if complete {
            crate::i18n::trf!("Log complete · sessions: {count}", count = s.sessions)
        } else {
            crate::i18n::tr("Some records are missing").to_owned()
        };
        let hover = if complete {
            crate::i18n::trf!("Read {records} records in {milliseconds} ms", records = s.records, milliseconds = s.elapsed_ms)
        } else {
            crate::i18n::trf!("Gaps: {gaps} · partial: {partial} · dropped: {dropped}", gaps = s.gaps, partial = s.partial, dropped = s.dropped)
        };
        foot_ui
            .label(RichText::new(text).size(12.0).color(if complete { HINT() } else { AMBER() }))
            .on_hover_text(hover);
        foot_ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let help = crate::chrome::icon_button(ui, Icon::Info, true, false, 26.0)
                .on_hover_text(crate::i18n::tr("About these records (guide)"));
            widgets.insert("activity-guide".into(), (&help).into());
            if help.clicked() {
                crate::guide::request(ui.ctx());
            }
        });
    }

    fn details(
        &mut self,
        ui: &mut Ui,
        widgets: &mut HashMap<String, UiControl>,
        rect: Rect,
        roomy: bool,
    ) {
        let Some(index) = self
            .selected
            .as_ref()
            .and_then(|k| self.snapshot.entries.iter().position(|e| e.key() == *k))
        else {
            return;
        };
        let key = self.snapshot.entries[index].key();
        let count = self.snapshot.entries[index].records.len();
        if self.steps.as_ref().is_none_or(|(k, n, _)| *k != key || *n != count) {
            self.steps = Some((key.clone(), count, step_rows(&self.snapshot.entries[index].records)));
            self.setup_open = false;
        }
        let e = &self.snapshot.entries[index];
        let rows = self.steps.as_ref().map_or(&[][..], |(.., rows)| &rows[..]);
        let copied = self
            .copied
            .as_ref()
            .is_some_and(|(key, at)| *key == e.key() && at.elapsed() < Duration::from_millis(1600));
        let mut copy = false;
        let mut show_records = None;
        let mut toggle_setup = false;
        let title = |ui: &mut Ui, copy: &mut bool, widgets: &mut HashMap<String, UiControl>| {
            status_icon(ui, e.status);
            ui.add(Label::new(RichText::new(&e.name).font(semibold(14.0))).truncate());
            crate::pages::pill(ui, e.status.label(), e.status.color());
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let r = crate::pages::slot_button(ui, Some(Icon::Copy), if copied { crate::i18n::tr("Copied") } else { crate::i18n::tr("Copy summary") }, 124.0, true)
                    .on_hover_text(crate::i18n::tr("Copies up to 200 structured records; no Git output, arguments or paths"));
                widgets.insert("activity-copy".into(), (&r).into());
                *copy = r.clicked();
            });
        };
        if roomy {
            crate::pages::card_bar(ui, |ui| title(ui, &mut copy, widgets));
        } else {
            Frame::new()
                .inner_margin(Margin {
                    left: 14,
                    right: 8,
                    top: 8,
                    bottom: 2,
                })
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        title(ui, &mut copy, widgets);
                    });
                });
        }
        let foot = 36.0;
        let bottom = rect.bottom() - foot;
        Frame::new()
            .inner_margin(Margin {
                left: 16,
                right: 16,
                top: if roomy { 12 } else { 6 },
                bottom: 0,
            })
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 18.0;
                    let steps = rows.iter().filter(|r| r.depth == 0 && !r.setup).count()
                        + usize::from(rows.iter().any(|r| r.setup));
                    let now = unix_ms();
                    ui.label(RichText::new(started(e.time, now)).color(MUTED()))
                        .on_hover_text(timestamp(e.time));
                    if let Some(ms) = e.duration {
                        ui.label(RichText::new(crate::i18n::trf!("Took {duration}", duration = duration(ms))).color(MUTED()));
                    }
                    ui.label(
                        RichText::new(crate::i18n::trf!("Steps: {count}", count = steps))
                            .color(MUTED()),
                    );
                });
                ui.add_space(if roomy { 12.0 } else { 6.0 });
                if let Some(problem) = &e.problem {
                    problem_box(ui, e.status, problem, roomy);
                }
                ui.add_space(if roomy { 12.0 } else { 6.0 });
                ui.horizontal(|ui| {
                    ui.label(RichText::new(crate::i18n::tr("STEPS")).font(semibold(11.0)).color(HINT()));
                    if self.records && e.records.len() > STEP_LIMIT {
                        let text = crate::i18n::trf!("First {limit} of {total} records", limit = STEP_LIMIT, total = e.records.len());
                        ui.label(RichText::new(text).size(12.0).color(HINT()));
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let items = [(crate::i18n::tr("Steps"), None, !self.records), (crate::i18n::tr("Records"), Some(e.records.len()), self.records)];
                        let responses = crate::pages::tab_segments(ui, &items);
                        for (r, (id, records)) in responses.into_iter().zip([("activity-view-steps", false), ("activity-view-records", true)]) {
                            widgets.insert(id.into(), (&r).into());
                            if r.clicked() {
                                show_records = Some(records);
                            }
                        }
                    });
                });
                ui.add_space(4.0);
            });
        let area = Rect::from_min_max(ui.cursor().min, pos2(rect.right(), bottom));
        let out = ui
            .scope_builder(UiBuilder::new().max_rect(area), |ui| {
                if self.records {
                    records_view(ui, e)
                } else {
                    steps_view(ui, e, rows, self.setup_open, &mut toggle_setup, widgets)
                }
            })
            .inner;
        widgets.insert(
            "activity-details".into(),
            UiControl {
                rect: out,
                enabled: true,
            },
        );
        let bar = Rect::from_min_max(pos2(rect.left(), bottom), rect.max);
        let mut foot_ui = card_foot(ui, bar);
        let (lock, _) = foot_ui.allocate_exact_size(vec2(12.0, 12.0), Sense::hover());
        icon(foot_ui.painter(), lock, Icon::Lock, HINT());
        if roomy {
            foot_ui.label(RichText::new(crate::i18n::tr("No Git output, arguments, paths or repository names.")).size(12.0).color(HINT()));
        }
        let short = e.session.get(..8).unwrap_or(&e.session).to_owned();
        let ids = crate::i18n::trf!("Request {request} · session {session}", request = e.request, session = short);
        foot_ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(RichText::new(ids).size(12.0).color(FAINT()))
                .on_hover_text(crate::i18n::trf!("Session {session}", session = e.session));
        });
        if copy {
            ui.ctx().copy_text(e.summary());
            self.copied = Some((e.key(), Instant::now()));
        }
        if copied {
            ui.ctx().request_repaint_after(Duration::from_millis(200));
        }
        if let Some(records) = show_records {
            self.records = records;
        }
        if toggle_setup {
            self.setup_open = !self.setup_open;
        }
    }
}

/// How long a read runs before it offers Stop.
const SLOW_READ: Duration = Duration::from_secs(1);
/// Below this width the page shows Operations and Details as tabs.
const NARROW: f32 = 900.0;
const GROUP: f32 = 28.0;
const ROW: f32 = 34.0;
/// A row that says why it needs a look under its name.
const TALL_ROW: f32 = 50.0;
const STEP_ROW: f32 = 32.0;
/// Steps listed for one operation; Copy summary still takes its first 200 records.
const STEP_LIMIT: usize = 500;
/// Steps that only get Git and the repository ready; they fold into one line.
const SETUP_STEPS: [&str; 4] = ["verify_git", "verify_runtime", "repository_discovery", "write_queue"];
// Keep every phase label inside the compact Details column.
const PHASES_PER_ROW: usize = 3;
fn phases() -> [(&'static str, Color32); 6] {
    [
        ("prepare_us", AMBER()),
        ("verify_us", MUTED()),
        ("spawn_us", VIOLET()),
        ("contain_us", TEAL()),
        ("execute_us", BLUE()),
        ("drain_us", GREEN()),
    ]
}
/// Result fields worth a chip beside a step; the raw names are what search finds.
const CHIPS: [&str; 21] = [
    "creation",
    "remote_added",
    "has_errors",
    "failure_code",
    "remote_outcome",
    "verification",
    "operation_state",
    "fetch_outcome",
    "update_outcome",
    "objects",
    "object_bytes",
    "checked",
    "total",
    "conflict_paths",
    "reason",
    "native_request",
    "tool",
    "phase",
    "queue_ms",
    "validation_ms",
    "input_delivered",
];

fn unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

fn duration(ms: u64) -> String {
    if ms >= 1000 {
        format!("{:.2} s", ms as f64 / 1000.0)
    } else {
        format!("{ms} ms")
    }
}

/// `ms` on this computer's clock, or as UTC if the system can't say.
pub(crate) fn local(ms: u64) -> rocket_git::LocalTime {
    rocket_git::local_time(ms).unwrap_or_else(|| civil(ms))
}

/// Days from 1970-01-01 to a calendar date.
fn day_number(t: &rocket_git::LocalTime) -> i64 {
    let (y, m, d) = (i64::from(t.year), i64::from(t.month), i64::from(t.day));
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// "Today", "Yesterday", a weekday within the last week, else the date.
pub(crate) fn day(ms: u64, now_ms: u64) -> String {
    let (then, now) = (local(ms), local(now_ms));
    let number = day_number(&then);
    match day_number(&now) - number {
        0 => crate::i18n::tr("Today").into(),
        1 => crate::i18n::tr("Yesterday").into(),
        2..7 => [crate::i18n::tr("Thursday"), crate::i18n::tr("Friday"), crate::i18n::tr("Saturday"), crate::i18n::tr("Sunday"), crate::i18n::tr("Monday"), crate::i18n::tr("Tuesday"), crate::i18n::tr("Wednesday")]
            [number.rem_euclid(7) as usize]
            .into(),
        _ => format!("{:04}-{:02}-{:02}", then.year, then.month, then.day),
    }
}

const MONTHS: [&str; 12] = [
    crate::i18n::mark("Jan"), crate::i18n::mark("Feb"), crate::i18n::mark("Mar"), crate::i18n::mark("Apr"), crate::i18n::mark("May"), crate::i18n::mark("Jun"), crate::i18n::mark("Jul"), crate::i18n::mark("Aug"), crate::i18n::mark("Sep"), crate::i18n::mark("Oct"), crate::i18n::mark("Nov"), crate::i18n::mark("Dec"),
];
/// A date in a list: "Today", a weekday this week, "2 Oct" this year, else "2 Oct 2025".
pub(crate) fn short_date(seconds: i64, now_ms: u64) -> String {
    let ms = u64::try_from(seconds).unwrap_or(0).saturating_mul(1000);
    let named = day(ms, now_ms);
    if !named.starts_with(|c: char| c.is_ascii_digit()) {
        return named;
    }
    let (then, now) = (local(ms), local(now_ms));
    let month = crate::i18n::tr(MONTHS[usize::from(then.month.clamp(1, 12)) - 1]);
    if then.year == now.year {
        format!("{} {month}", then.day)
    } else {
        format!("{} {month} {}", then.day, then.year)
    }
}
/// Milliseconds since 1970 now.
pub(crate) fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

/// "00:59" on this computer's clock.
pub(crate) fn clock(ms: u64) -> String {
    let t = local(ms);
    format!("{:02}:{:02}", t.hour, t.minute)
}

/// "Today at 00:59:43" on this computer's clock.
pub(crate) fn started(ms: u64, now_ms: u64) -> String {
    let t = local(ms);
    crate::i18n::trf!("{day} at {time}", day = day(ms, now_ms), time = format!("{:02}:{:02}:{:02}", t.hour, t.minute, t.second))
}

/// The row at a card's foot, its content from the left.
fn card_foot(ui: &mut Ui, rect: Rect) -> Ui {
    ui.painter().hline(rect.x_range(), rect.top() + 0.5, Stroke::new(1.0, CARD_LINE()));
    let inner = Rect::from_min_max(rect.min + vec2(14.0, 0.0), rect.max - vec2(8.0, 0.0));
    let mut foot = ui.new_child(UiBuilder::new().max_rect(inner).layout(Layout::left_to_right(Align::Center)));
    foot.spacing_mut().item_spacing.x = 7.0;
    foot
}

/// Content `height` tall, centred in `rect`.
fn centred(ui: &mut Ui, rect: Rect, height: f32, content: impl FnOnce(&mut Ui)) {
    let top = ((rect.height() - height) / 2.0).max(16.0);
    ui.vertical_centered(|ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        ui.add_space(top);
        content(ui);
    });
}

/// A round toggle such as This session only, with an optional count.
pub(crate) fn chip(ui: &mut Ui, label: &str, count: Option<usize>, on: bool, height: f32) -> Response {
    let ink = if on { BLUE() } else { MUTED() };
    let size = if height < CONTROL { 12.0 } else { 12.5 };
    let mut job = text::LayoutJob::default();
    job.append(label, 0.0, TextFormat::simple(FontId::proportional(size), ink));
    if let Some(count) = count {
        let tint = if on { BLUE() } else { HINT() };
        job.append(&format!("  {}", crate::pages::number_text(count)), 0.0, TextFormat::simple(FontId::proportional(size), tint));
    }
    let chip = ui
        .scope(|ui| {
            ui.spacing_mut().interact_size.y = height;
            ui.add(
                Button::new(job)
                    .fill(if on { ACCENT_SOFT() } else { Color32::TRANSPARENT })
                    .stroke(Stroke::new(1.0, if on { BLUE() } else { BORDER() }))
                    .corner_radius(height / 2.0)
                    .min_size(vec2(0.0, height)),
            )
        })
        .inner;
    chip.widget_info(|| WidgetInfo::selected(Role::CheckBox, true, on, label));
    chip
}

/// The round mark of a status: check, warning, cancelled or dotted (incomplete).
fn status_mark(painter: &Painter, center: Pos2, status: Status) {
    let color = if status == Status::Succeeded { GREEN().gamma_multiply(0.72) } else { status.color() };
    let stroke = Stroke::new(1.4, color);
    match status {
        Status::Succeeded => {
            painter.circle_stroke(center, 6.5, stroke);
            painter.line_segment([center + vec2(-3.0, 0.2), center + vec2(-0.8, 2.4)], stroke);
            painter.line_segment([center + vec2(-0.8, 2.4), center + vec2(3.2, -2.2)], stroke);
        }
        Status::Attention => icon(painter, Rect::from_center_size(center, vec2(14.0, 14.0)), Icon::Warning, color),
        Status::Cancelled => {
            painter.circle_stroke(center, 6.5, stroke);
            painter.line_segment([center + vec2(-3.0, 0.0), center + vec2(3.0, 0.0)], stroke);
        }
        Status::Incomplete => {
            for k in 0..12 {
                if k % 2 == 0 {
                    let a = std::f32::consts::TAU * k as f32 / 12.0;
                    let b = std::f32::consts::TAU * (k as f32 + 1.0) / 12.0;
                    painter.line_segment(
                        [center + vec2(a.cos(), a.sin()) * 6.5, center + vec2(b.cos(), b.sin()) * 6.5],
                        stroke,
                    );
                }
            }
        }
    }
}

/// A status mark as a widget in a row.
fn status_icon(ui: &mut Ui, status: Status) {
    let (rect, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
    status_mark(ui.painter(), rect.center(), status);
}

fn paint_group(ui: &Ui, rect: Rect, label: &str, count: usize, open: bool, hovered: bool) {
    let painter = ui.painter();
    painter.rect_filled(rect, 0.0, if hovered { WELL_HOT() } else { WELL() });
    painter.hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0, WELL_LINE()));
    let mark = Rect::from_center_size(pos2(rect.left() + 16.0, rect.center().y), vec2(11.0, 11.0));
    icon(painter, mark, if open { Icon::Chevron } else { Icon::ChevronRight }, HINT());
    painter.text(
        pos2(rect.left() + 28.0, rect.center().y),
        Align2::LEFT_CENTER,
        label.to_uppercase(),
        semibold(11.0),
        HINT(),
    );
    painter.text(
        pos2(rect.right() - 14.0, rect.center().y),
        Align2::RIGHT_CENTER,
        crate::pages::number_text(count),
        FontId::proportional(12.0),
        HINT(),
    );
}

fn paint_entry(ui: &Ui, rect: Rect, e: &Entry, selected: bool, hovered: bool) {
    let painter = ui.painter();
    if selected {
        painter.rect_filled(rect, 0.0, SELECTED());
        painter.rect_filled(Rect::from_min_size(rect.min, vec2(3.0, rect.height())), 0.0, BLUE());
    } else if hovered {
        painter.rect_filled(rect, 0.0, HOVER());
    }
    painter.hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0, WELL_LINE()));
    status_mark(painter, pos2(rect.left() + 22.0, rect.center().y), e.status);
    let right = rect.right() - 14.0;
    let time = painter.layout_no_wrap(clock(e.time), FontId::proportional(12.0), HINT());
    let spent = e.duration.map_or_else(|| "—".into(), duration);
    let spent = painter.layout_no_wrap(spent, FontId::proportional(12.0), MUTED());
    let spent_left = right - 46.0 - spent.size().x;
    let left = rect.left() + 40.0;
    let room = (spent_left - left - 10.0).max(0.0);
    painter.galley(pos2(right - time.size().x, rect.center().y - time.size().y / 2.0), time, HINT());
    painter.galley(pos2(spent_left, rect.center().y - spent.size().y / 2.0), spent, MUTED());
    let cancelled = e.status == Status::Cancelled;
    let (name, font, ink) = if e.tall() {
        (e.name.clone(), semibold(13.0), TEXT())
    } else if cancelled {
        (crate::i18n::trf!("{operation} · cancelled", operation = e.name), FontId::proportional(13.0), HINT())
    } else if e.read {
        (e.name.clone(), FontId::proportional(13.0), HINT())
    } else {
        (e.name.clone(), FontId::proportional(13.0), TEXT())
    };
    let mut job = text::LayoutJob::simple_singleline(name, font, ink);
    job.wrap = text::TextWrapping::truncate_at_width(room);
    let name = painter.layout_job(job);
    let middle = if e.tall() { rect.top() + 16.0 } else { rect.center().y };
    painter.galley(pos2(left, middle - name.size().y / 2.0), name, ink);
    if let Some(problem) = e.problem.as_ref().filter(|_| e.tall()) {
        let font = FontId::proportional(12.0);
        let whole = painter.layout_no_wrap(problem.headline.clone(), font.clone(), AMBER()).size().x <= room;
        let words = if whole { &problem.headline } else { &problem.short };
        let mut job = text::LayoutJob::simple_singleline(words.clone(), font, AMBER());
        job.wrap = text::TextWrapping::truncate_at_width(room);
        let line = painter.layout_job(job);
        painter.galley(pos2(left, rect.top() + 34.0 - line.size().y / 2.0), line, AMBER());
    }
}

/// Why an operation needs a look, in Rocket's words where it has them.
#[derive(Clone, Debug)]
struct Problem {
    headline: String,
    /// One line for the list.
    short: String,
    advice: Option<&'static str>,
    /// The fields that say so.
    chips: Vec<String>,
    /// Where it stopped, such as "at git fetch · exit 128".
    place: Option<String>,
}

fn problem(status: Status, records: &[Value]) -> Option<Problem> {
    let fields = |keys: &[&str]| {
        let mut chips: Vec<String> = Vec::new();
        for r in records {
            for key in keys {
                if let Some(v) = r.get(*key).filter(|v| !v.is_null()) {
                    let value = v.as_str().map_or_else(|| v.to_string(), str::to_owned);
                    let chip = if *key == "failure_code" { value } else { format!("{key}: {value}") };
                    if !chips.contains(&chip) {
                        chips.push(chip);
                    }
                }
            }
        }
        chips.truncate(6);
        chips
    };
    match status {
        Status::Succeeded => None,
        Status::Cancelled => Some(Problem {
            headline: crate::i18n::tr("Cancelled.").into(),
            short: crate::i18n::tr("Cancelled.").into(),
            advice: None,
            chips: fields(&["cancel_to_stop_us"]),
            place: None,
        }),
        Status::Incomplete => Some(Problem {
            headline: crate::i18n::tr("The records end before this operation finished, so its outcome cannot be confirmed.").into(),
            short: crate::i18n::tr("The records end before it finished.").into(),
            advice: None,
            chips: Vec::new(),
            place: None,
        }),
        Status::Attention => {
            let at = records.iter().position(failed_result);
            let code = records
                .iter()
                .map(|r| label(r, "failure_code"))
                .find(|c| !c.is_empty() && *c != "none");
            let text = code.and_then(rocket_git::failure_text);
            let outcome = records
                .iter()
                .map(|r| label(r, "outcome"))
                .find(|o| !o.is_empty() && !matches!(*o, "ok" | "exited" | "cancelled" | "clean"));
            let headline = match (text, code, outcome) {
                (Some(text), ..) => text.summary.unwrap_or(text.guidance).to_owned(),
                (None, Some(code), _) => format!("{}.", spelled(code)),
                (None, None, Some(outcome)) => format!("{}.", spelled(outcome)),
                (None, None, None) => crate::i18n::tr("This operation needs a look.").to_owned(),
            };
            let place = at.and_then(|at| {
                let r = &records[at];
                if let Some(git) = failed_command(records) {
                    let exit = git["exit_code"].as_i64().unwrap_or_default();
                    Some(crate::i18n::trf!("at git {operation} · exit {exit}", operation = label(git, "operation"), exit))
                } else if label(r, "event") == "git_end" {
                    Some(crate::i18n::trf!("at git {operation}", operation = label(r, "operation")))
                } else {
                    Some(label(r, "step")).filter(|s| !s.is_empty()).map(|s| crate::i18n::trf!("at {step}", step = spelled(s)))
                }
            });
            Some(Problem {
                short: headline.split_inclusive(". ").next().unwrap_or_default().trim_end().to_owned(),
                headline,
                advice: text.filter(|t| t.summary.is_some()).map(|t| t.guidance),
                chips: fields(&["failure_code", "remote_outcome", "conflict_paths", "operation_state", "reason"]),
                place,
            })
        }
    }
}

fn problem_box(ui: &mut Ui, status: Status, problem: &Problem, roomy: bool) {
    let color = status.color();
    Frame::new()
        .fill(color.gamma_multiply(0.07))
        .stroke(Stroke::new(1.0, color.gamma_multiply(0.3)))
        .corner_radius(8)
        .inner_margin(Margin::symmetric(14, if roomy { 11 } else { 8 }))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 6.0;
            ui.add(Label::new(RichText::new(&problem.headline).font(semibold(13.0))).wrap());
            if let Some(advice) = problem.advice.filter(|_| roomy) {
                ui.add(Label::new(RichText::new(advice).color(TEXT_SOFT())).wrap());
            }
            if !problem.chips.is_empty() || problem.place.is_some() {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(8.0, 6.0);
                    for chip in &problem.chips {
                        chip_label(ui, chip, status == Status::Attention);
                    }
                    if let Some(place) = &problem.place {
                        ui.label(RichText::new(place).size(12.0).color(HINT()));
                    }
                });
            }
        });
}

fn chip_label(ui: &mut Ui, text: &str, warn: bool) {
    let color = if warn { AMBER() } else { MUTED() };
    let galley = ui.painter().layout_no_wrap(text.into(), FontId::monospace(11.5), color);
    let (rect, _) = ui.allocate_exact_size(vec2(galley.size().x + 12.0, 20.0), Sense::hover());
    ui.painter().rect_filled(rect, 4.0, if warn { color.gamma_multiply(0.14) } else { RAISED() });
    ui.painter().galley(pos2(rect.left() + 6.0, rect.center().y - galley.size().y / 2.0), galley, color);
}

/// Whether a record's own result is a failure. A Git command's exit code alone isn't:
/// many commands answer a question that way, such as a config key that isn't set.
fn failed_result(r: &Value) -> bool {
    let (outcome, code) = (label(r, "outcome"), label(r, "failure_code"));
    (!code.is_empty() && code != "none")
        || (!outcome.is_empty()
            && !matches!(outcome, "ok" | "exited" | "cancelled")
            && !(label(r, "event") == "commit_prediction" && outcome == "clean"))
}

/// The Git command a failure came from: the last one before it with the exit code the
/// failure reports. A failure with no exit code didn't come from a command.
fn failed_command(records: &[Value]) -> Option<&Value> {
    let at = records.iter().position(failed_result)?;
    let exit = records[at]["exit_code"].as_i64().filter(|c| *c != 0)?;
    records[..at]
        .iter()
        .rev()
        .find(|r| label(r, "event") == "git_end" && r["exit_code"].as_i64() == Some(exit))
}

/// Whether a record looks failed in the raw timeline: a failed outcome, an exit code or a
/// failure code.
fn step_failed(r: &Value) -> bool {
    let outcome = label(r, "outcome");
    outcome.contains("fail")
        || outcome.contains("error")
        || r["exit_code"].as_i64().is_some_and(|c| c != 0)
        || r.get("failure_code").is_some_and(|v| !v.is_null())
}

/// One step or Git command of an operation: its start and end records as one line.
#[derive(Clone, Debug, PartialEq, Eq)]
struct StepRow {
    depth: usize,
    name: String,
    git: bool,
    setup: bool,
    state: StepState,
    elapsed: Option<u64>,
    exit: Option<i64>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StepState {
    Running,
    Done,
    Failed,
    Cancelled,
}

/// Pairs start and end records by id, nesting commands under the steps still open. Some
/// commands run side by side, so a start and its end need not be neighbours. A record
/// with neither stays in Records only.
fn step_rows(records: &[Value]) -> Vec<StepRow> {
    let failing = failed_command(records).map(|r| number(r, "id"));
    let mut rows: Vec<StepRow> = Vec::new();
    let mut steps: Vec<(u64, usize)> = Vec::new();
    let mut commands: Vec<(u64, usize)> = Vec::new();
    let row = |r: &Value, depth: usize, git: bool, setup: bool| StepRow {
        depth,
        name: if git {
            format!("git {}", label(r, "operation"))
        } else if label(r, "step").is_empty() {
            spelled(label(r, "operation"))
        } else {
            spelled(label(r, "step"))
        },
        git,
        setup: setup || SETUP_STEPS.contains(&label(r, "step")),
        state: StepState::Running,
        elapsed: None,
        exit: None,
    };
    let finish = |row: &mut StepRow, r: &Value| {
        let failed = failed_result(r) || (row.git && failing == Some(number(r, "id")));
        row.elapsed = r["elapsed_ms"].as_u64();
        row.exit = r["exit_code"].as_i64().filter(|c| *c != 0 && failed);
        row.state = if label(r, "outcome") == "cancelled" {
            StepState::Cancelled
        } else if failed {
            StepState::Failed
        } else {
            StepState::Done
        };
    };
    for r in records {
        let event = label(r, "event");
        let id = number(r, "id");
        let git = matches!(event, "git_start" | "git_end");
        if matches!(event, "step_start" | "git_start") {
            let setup = steps.first().is_some_and(|(_, at)| rows[*at].setup);
            rows.push(row(r, steps.len(), git, setup));
            if git {
                commands.push((id, rows.len() - 1));
            } else {
                steps.push((id, rows.len() - 1));
            }
        } else if matches!(event, "step_end" | "git_end") {
            let open = if git { &mut commands } else { &mut steps };
            match open.iter().rposition(|(open_id, _)| *open_id == id) {
                Some(found) => {
                    let (_, at) = open.remove(found);
                    finish(&mut rows[at], r);
                }
                // Its start was rotated away or cut off.
                None => {
                    let depth = steps.len();
                    let mut lone = row(r, depth, git, false);
                    finish(&mut lone, r);
                    rows.push(lone);
                }
            }
        }
    }
    // A failed setup step stays in the open.
    if rows.iter().any(|r| r.setup && r.state != StepState::Done) {
        for r in &mut rows {
            r.setup = false;
        }
    }
    rows
}

/// The steps as a list, setup folded into one line.
fn steps_view(
    ui: &mut Ui,
    e: &Entry,
    rows: &[StepRow],
    setup_open: bool,
    toggle: &mut bool,
    widgets: &mut HashMap<String, UiControl>,
) -> Rect {
    let setup_ms: u64 = rows.iter().filter(|r| r.setup && r.depth == 0).filter_map(|r| r.elapsed).sum();
    let has_setup = rows.iter().any(|r| r.setup);
    let shown: Vec<&StepRow> = rows.iter().filter(|r| setup_open || !r.setup).collect();
    let count = shown.len() + usize::from(has_setup);
    ScrollArea::vertical()
        .id_salt(("activity-history-steps", e.key()))
        .auto_shrink([false, false])
        .show_rows(ui, STEP_ROW, count, |ui, range| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for k in range {
                let width = ui.available_width();
                if has_setup && k == 0 {
                    let (rect, response) = ui.allocate_exact_size(vec2(width, STEP_ROW), Sense::click());
                    if response.hovered() {
                        ui.painter().rect_filled(rect, 0.0, HOVER());
                    }
                    let chevron = Rect::from_center_size(pos2(rect.left() + 20.0, rect.center().y), vec2(11.0, 11.0));
                    icon(ui.painter(), chevron, if setup_open { Icon::Chevron } else { Icon::ChevronRight }, HINT());
                    status_mark(ui.painter(), pos2(rect.left() + 38.0, rect.center().y), Status::Succeeded);
                    ui.painter().text(
                        pos2(rect.left() + 54.0, rect.center().y),
                        Align2::LEFT_CENTER,
                        crate::i18n::tr("Setup · find Git, open the repository"),
                        FontId::proportional(13.0),
                        HINT(),
                    );
                    ui.painter().text(
                        pos2(rect.right() - 16.0, rect.center().y),
                        Align2::RIGHT_CENTER,
                        duration(setup_ms),
                        FontId::proportional(12.0),
                        MUTED(),
                    );
                    response.widget_info(|| WidgetInfo::selected(Role::Button, true, setup_open, crate::i18n::tr("Setup")));
                    widgets.insert("activity-setup".into(), (&response).into());
                    *toggle |= response.clicked();
                    continue;
                }
                let row = shown[k - usize::from(has_setup)];
                let (rect, _) = ui.allocate_exact_size(vec2(width, STEP_ROW), Sense::hover());
                paint_step_row(ui, rect, row);
            }
        })
        .inner_rect
}

fn paint_step_row(ui: &Ui, rect: Rect, row: &StepRow) {
    let painter = ui.painter();
    let failed = row.state == StepState::Failed;
    if failed {
        painter.rect_filled(rect, 0.0, RED().gamma_multiply(0.08));
    }
    let center = pos2(rect.left() + 38.0 + row.depth as f32 * 20.0, rect.center().y);
    match row.state {
        StepState::Done => status_mark(painter, center, Status::Succeeded),
        StepState::Cancelled => status_mark(painter, center, Status::Cancelled),
        StepState::Running => status_mark(painter, center, Status::Incomplete),
        StepState::Failed => {
            let stroke = Stroke::new(1.4, RED());
            painter.circle_stroke(center, 6.5, stroke);
            painter.line_segment([center + vec2(-2.6, -2.6), center + vec2(2.6, 2.6)], stroke);
            painter.line_segment([center + vec2(2.6, -2.6), center + vec2(-2.6, 2.6)], stroke);
        }
    }
    let mut right = rect.right() - 16.0;
    if let Some(ms) = row.elapsed {
        let galley = painter.layout_no_wrap(duration(ms), FontId::proportional(12.0), MUTED());
        right -= galley.size().x;
        painter.galley(pos2(right, rect.center().y - galley.size().y / 2.0), galley, MUTED());
        right -= 10.0;
    }
    if let Some(exit) = row.exit {
        let galley = painter.layout_no_wrap(format!("exit {exit}"), FontId::monospace(11.5), RED());
        let chip = Rect::from_min_size(pos2(right - galley.size().x - 12.0, rect.center().y - 10.0), vec2(galley.size().x + 12.0, 20.0));
        painter.rect_filled(chip, 4.0, RED().gamma_multiply(0.14));
        painter.galley(pos2(chip.left() + 6.0, chip.center().y - galley.size().y / 2.0), galley, RED());
        right = chip.left() - 10.0;
    }
    let left = center.x + 16.0;
    let (font, ink) = match (row.git, failed) {
        (true, true) => (FontId::monospace(12.5), RED()),
        (true, false) => (FontId::monospace(12.5), TEXT_SOFT()),
        (false, _) => (FontId::proportional(13.0), TEXT()),
    };
    let mut job = text::LayoutJob::simple_singleline(row.name.clone(), font, ink);
    job.wrap = text::TextWrapping::truncate_at_width((right - left).max(0.0));
    let name = painter.layout_job(job);
    painter.galley(pos2(left, rect.center().y - name.size().y / 2.0), name, ink);
}

/// Every record as a timeline; only the visible ones are laid out.
fn records_view(ui: &mut Ui, e: &Entry) -> Rect {
    let records = &e.records[..e.records.len().min(STEP_LIMIT)];
    let heights: Vec<f32> = records.iter().map(step_height).collect();
    let mut tops = Vec::with_capacity(heights.len());
    let mut total = 0.0;
    for h in &heights {
        tops.push(total);
        total += h;
    }
    ScrollArea::vertical()
        .id_salt(("activity-history-details", e.key()))
        .auto_shrink([false, false])
        .show_viewport(ui, |ui, viewport| {
            ui.set_height(total);
            let origin = ui.min_rect().min + vec2(16.0, 0.0);
            let width = ui.available_width() - 32.0;
            let first = tops.partition_point(|t| *t + 120.0 < viewport.min.y);
            for (k, record) in records.iter().enumerate().skip(first) {
                if tops[k] > viewport.max.y {
                    break;
                }
                let rect = Rect::from_min_size(origin + vec2(0.0, tops[k]), vec2(width, heights[k]));
                paint_step(ui, rect, record, k + 1 == records.len());
            }
        })
        .inner_rect
}

fn step_chips(r: &Value) -> Vec<(String, bool)> {
    let mut chips = Vec::new();
    let outcome = label(r, "outcome");
    if !outcome.is_empty() {
        chips.push((outcome.to_owned(), step_failed(r)));
    }
    if let Some(code) = r["exit_code"].as_i64() {
        chips.push((format!("exit {code}"), code != 0));
    }
    if let Some(us) = r["cancel_to_stop_us"].as_u64() {
        chips.push((format!("stop {:.1} ms", us as f64 / 1000.0), false));
    }
    for key in CHIPS {
        if let Some(v) = r.get(key).filter(|v| !v.is_null()) {
            let value = v.as_str().map_or_else(|| v.to_string(), str::to_owned);
            chips.push((format!("{key}: {value}"), key == "failure_code"));
        }
    }
    chips
}

fn step_phases(r: &Value) -> Vec<(&'static str, u64, Color32)> {
    phases()
        .iter()
        .filter_map(|(key, color)| r[*key].as_u64().map(|us| (*key, us, *color)))
        .collect()
}

fn step_height(r: &Value) -> f32 {
    let mut height = 30.0;
    if !step_chips(r).is_empty() {
        height += 24.0;
    }
    let count = step_phases(r).len();
    if count > 0 {
        height += 14.0 + 18.0 * count.div_ceil(PHASES_PER_ROW) as f32;
    }
    height
}

/// One step of the timeline: a dot on the line, the event and step, its timing, the
/// result fields as chips and, for a Git process, where its time went.
fn paint_step(ui: &Ui, rect: Rect, r: &Value, last: bool) {
    let painter = ui.painter();
    let event = label(r, "event");
    let dot = pos2(rect.left() + 6.0, rect.top() + 11.0);
    if !last {
        painter.vline(dot.x, (dot.y + 6.0)..=(rect.bottom() + 5.0), Stroke::new(2.0, CARD_LINE()));
    }
    let failed = step_failed(r);
    let cancelled = label(r, "outcome") == "cancelled";
    if event.ends_with("_start") {
        painter.circle(dot, 4.5, BG(), Stroke::new(2.0, BLUE()));
    } else if failed {
        painter.circle_filled(dot, 4.5, RED());
    } else if cancelled {
        painter.circle_filled(dot, 4.5, FAINT());
    } else if event.ends_with("_end") || event == "git_version" {
        painter.circle_filled(dot, 4.5, GREEN());
    } else {
        painter.circle(dot, 4.5, BG(), Stroke::new(2.0, RING()));
    }
    let x = rect.left() + 22.0;
    let name = painter.layout_no_wrap(format!("#{} {event}", number(r, "seq")), FontId::monospace(12.5), TEXT());
    let name_width = name.size().x;
    painter.galley(pos2(x, dot.y - name.size().y / 2.0), name, TEXT());
    let step = if label(r, "step").is_empty() { label(r, "operation") } else { label(r, "step") };
    let timing = r["elapsed_ms"].as_u64().map(duration).unwrap_or_default();
    let timing_galley = painter.layout_no_wrap(timing, FontId::proportional(12.0), MUTED());
    let timing_left = rect.right() - timing_galley.size().x;
    let mut job = text::LayoutJob::simple_singleline(step.to_owned(), FontId::proportional(12.5), HINT());
    job.wrap = text::TextWrapping::truncate_at_width((timing_left - x - name_width - 20.0).max(0.0));
    let step = painter.layout_job(job);
    painter.galley(pos2(x + name_width + 8.0, dot.y - step.size().y / 2.0), step, HINT());
    painter.galley(pos2(timing_left, dot.y - timing_galley.size().y / 2.0), timing_galley, MUTED());
    let mut y = rect.top() + 24.0;
    let chips = step_chips(r);
    if !chips.is_empty() {
        let mut cx = x;
        for (text, bad) in chips {
            let color = if bad { RED() } else { MUTED() };
            let galley = painter.layout_no_wrap(text, FontId::monospace(11.5), color);
            let chip = Rect::from_min_size(pos2(cx, y), vec2(galley.size().x + 12.0, 19.0));
            if chip.right() > rect.right() {
                break;
            }
            painter.rect_filled(chip, 4.0, if bad { RED().gamma_multiply(0.12) } else { RAISED() });
            painter.galley(pos2(chip.left() + 6.0, chip.center().y - galley.size().y / 2.0), galley, color);
            cx = chip.right() + 5.0;
        }
        y += 24.0;
    }
    let phases = step_phases(r);
    if !phases.is_empty() {
        // Where a Git process spent its time, as one proportional bar.
        let total: u64 = phases.iter().map(|(_, us, _)| us).sum::<u64>().max(1);
        let bar = Rect::from_min_size(pos2(x, y + 2.0), vec2((rect.right() - x).min(460.0), 6.0));
        let mut left = bar.left();
        for (_, us, color) in &phases {
            let width = (bar.width() * *us as f32 / total as f32).max(3.0);
            let segment = Rect::from_min_size(pos2(left, bar.top()), vec2(width.min(bar.right() - left).max(0.0), bar.height()));
            painter.rect_filled(segment, 2.0, *color);
            left = (segment.right() + 1.0).min(bar.right());
        }
        let mut cx = x;
        let mut cy = y + 19.0;
        for (index, (key, us, color)) in phases.iter().enumerate() {
            if index > 0 && index % PHASES_PER_ROW == 0 {
                cx = x;
                cy += 18.0;
            }
            let text = format!("{} {}", key.trim_end_matches("_us"), if *us >= 1000 { format!("{:.1} ms", *us as f64 / 1000.0) } else { format!("{us} µs") });
            painter.rect_filled(Rect::from_center_size(pos2(cx + 4.0, cy), vec2(7.0, 7.0)), 2.0, *color);
            let galley = painter.layout_no_wrap(text, FontId::proportional(11.5), MUTED());
            let width = galley.size().x;
            painter.galley(pos2(cx + 11.0, cy - galley.size().y / 2.0), galley, MUTED());
            cx += width + 24.0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn localized_statuses_preserve_exported_diagnostic_values() {
        let rows = vec![row(1, 1, "request_start", json!({})), row(2, 1, "request_end", json!({"outcome":"ok"}))];
        let snapshot = aggregate(rows, &CancellationToken::new()).unwrap();
        let entry = &snapshot.entries[0];
        let expected = entry.summary();
        for language in crate::i18n::Language::ALL.into_iter().skip(1) {
            crate::i18n::scoped(language, || {
                assert_ne!(Status::Succeeded.label(), "Succeeded", "{language:?}");
                assert_eq!(Status::Succeeded.code(), "Succeeded");
                assert_eq!(Filter::Attention.code(), "Needs attention");
                assert_eq!(entry.summary(), expected, "{language:?}");
            });
        }
    }
    #[test]
    fn github_publish_partial_outcomes_keep_safe_phases_and_need_attention() {
        for creation in ["unknown", "rejected", "created"] {
            let projected = project(row(2, 1, "github_publish_outcome", json!({
                "operation":"github-publish", "level":"warn", "creation":creation,
                "remote_added":false, "has_errors":true, "remote_outcome":"not_attempted",
                "url":"https://github.com/private/secret", "token":"private-value"
            }))).unwrap();
            assert_eq!(projected["creation"], creation);
            assert_eq!(projected["remote_added"], false);
            assert_eq!(projected["has_errors"], true);
            assert!(projected.get("url").is_none() && projected.get("token").is_none());
            let entries = aggregate(vec![row(1,1,"request_start",json!({})), projected,
                row(3,1,"request_end",json!({"outcome":"ok"}))], &CancellationToken::new()).unwrap();
            assert_eq!(entries.entries[0].status, Status::Attention);
        }
    }
    #[test]
    fn prediction_outcomes_are_classified_without_hiding_other_errors() {
        let c = CancellationToken::new();
        for (outcome, status) in [
            ("clean", Status::Succeeded),
            ("conflicts", Status::Attention),
            ("empty", Status::Attention),
            ("unavailable", Status::Attention),
        ] {
            let rows = vec![
                row(1, 1, "request_start", json!({})),
                row(2, 1, "commit_prediction", json!({"outcome":outcome})),
                row(3, 1, "request_end", json!({"outcome":"ok"})),
            ];
            assert_eq!(aggregate(rows, &c).unwrap().entries[0].status, status);
        }
        for event in ["step_end", "request_end"] {
            let rows = vec![
                row(1, 1, "request_start", json!({})),
                row(2, 1, "commit_prediction", json!({"outcome":"clean"})),
                row(3, 1, event, json!({"outcome":"git_failed"})),
                row(4, 1, "request_end", json!({"outcome":"ok"})),
            ];
            assert_eq!(
                aggregate(rows, &c).unwrap().entries[0].status,
                Status::Attention
            );
        }
        // "clean" is successful only for the new prediction event, not an arbitrary result.
        let rows = vec![
            row(1, 1, "request_start", json!({})),
            row(2, 1, "step_end", json!({"outcome":"clean"})),
            row(3, 1, "request_end", json!({"outcome":"ok"})),
        ];
        assert_eq!(
            aggregate(rows, &c).unwrap().entries[0].status,
            Status::Attention
        );
    }
    #[test]
    fn prediction_details_keep_counts_and_fixed_reasons_without_payloads() {
        let safe = project(row(
            1,
            1,
            "commit_prediction",
            json!({
                "operation":"commit-prediction", "outcome":"unavailable",
                "checked":2, "total":4, "conflict_paths":1,
                "reason":"merge_attributes", "paths":["PRIVATE/path"], "message":"PRIVATE"
            }),
        ))
        .unwrap();
        assert_eq!(safe["checked"], 2);
        assert_eq!(safe["total"], 4);
        assert_eq!(safe["conflict_paths"], 1);
        assert_eq!(safe["reason"], "merge_attributes");
        assert!(!safe.to_string().contains("PRIVATE"));
        let invalid = project(row(
            2,
            1,
            "commit_prediction",
            json!({
                "reason":"private-path", "checked":"PRIVATE", "total":-1,
                "conflict_paths":"PRIVATE"
            }),
        ))
        .unwrap();
        assert_eq!(invalid["reason"], "unrecognized");
        assert!(invalid["checked"].is_null());
        assert!(invalid["total"].is_null());
        assert!(invalid["conflict_paths"].is_null());
    }
    fn row(seq: u64, request: u64, event: &str, extra: Value) -> Value {
        let mut r = json!({"schema":1,"session":"abc-1-1","seq":seq,"request":request,"time_unix_ms":1792395080000u64,"event":event,"operation":"lfs-push","dropped_before":0});
        for (k, v) in extra.as_object().unwrap() {
            r[k] = v.clone()
        }
        r
    }
    fn root() -> PathBuf {
        let p = std::env::temp_dir().join(format!(
            "rocket-activity-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&p).unwrap();
        p
    }
    fn write(p: &Path, records: &[Value]) {
        fs::write(
            p,
            records
                .iter()
                .map(|r| r.to_string() + "\n")
                .collect::<String>(),
        )
        .unwrap();
    }
    #[test]
    fn process_stop_timing_survives_projection_without_private_fields() {
        let record = project(row(
            2,
            1,
            "git_end",
            json!({
                "outcome":"cancelled", "elapsed_ms":8000, "cancel_to_stop_us":1250,
                "stderr":"PRIVATE credential", "url":"PRIVATE destination"
            }),
        ))
        .unwrap();
        assert_eq!(record["cancel_to_stop_us"], 1250);
        assert_eq!(record["elapsed_ms"], 8000);
        assert!(!record.to_string().contains("PRIVATE"));
        let invalid =
            project(row(2, 1, "git_end", json!({"cancel_to_stop_us":"PRIVATE"}))).unwrap();
        assert!(invalid.get("cancel_to_stop_us").is_none());
    }

    #[test]
    fn all_process_phase_labels_fit_narrow_diagnostic_cards() {
        for width in [380.0, 530.0, 780.0] {
            let ctx = Context::default();
            let record = row(2, 1, "git_end", json!({
                "prepare_us":1200,"verify_us":500,"spawn_us":13000,
                "contain_us":4000,"execute_us":40000,"drain_us":25,
                "elapsed_ms":60,"outcome":"exited","exit_code":0
            }));
            let mut bounds = Rect::NOTHING;
            let mut output = ctx.run_ui(RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(width + 40.0, 250.0))), ..Default::default() }, |ui| {
                bounds = ui.allocate_exact_size(vec2(width, step_height(&record)), Sense::hover()).0;
                paint_step(ui, bounds, &record, true);
            });
            output.textures_delta.clear();
            let labels: Vec<_> = output.shapes.iter().filter_map(|s| match &s.shape {
                Shape::Text(t) if phases().iter().any(|(key, _)| t.galley.job.text.starts_with(&format!("{} ", key.trim_end_matches("_us")))) => {
                    Some(Rect::from_min_size(t.pos, t.galley.size()))
                }
                _ => None,
            }).collect();
            assert_eq!(labels.len(), 6);
            for (i, label) in labels.iter().enumerate() {
                assert!(bounds.contains_rect(*label), "phase label outside {width}px card: {label:?}");
                assert!(labels[..i].iter().all(|previous| !previous.intersects(*label)), "overlapping timing labels");
            }
        }
    }

    #[test]
    fn process_phases_survive_projection_as_numbers_only() {
        let keys = [
            "prepare_us",
            "verify_us",
            "spawn_us",
            "contain_us",
            "execute_us",
            "drain_us",
        ];
        for key in keys {
            let valid = project(row(
                2,
                1,
                "git_end",
                json!({key: 123, "arguments":"PRIVATE"}),
            ))
            .unwrap();
            assert_eq!(valid[key], 123);
            assert!(step_phases(&valid).iter().any(|(phase, us, _)| *phase == key && *us == 123));
            assert!(!valid.to_string().contains("PRIVATE"));
            for value in [
                json!("PRIVATE"),
                json!(-1),
                json!({"path":"PRIVATE"}),
                json!(1.2),
            ] {
                let invalid = project(row(2, 1, "git_end", json!({key: value}))).unwrap();
                assert!(invalid.get(key).is_none());
            }
        }
    }

    #[test]
    fn mcp_timing_is_a_complete_searchable_operation_without_payloads() {
        let safe = project(row(
            1,
            7,
            "mcp_request",
            json!({
                "operation":"mcp-ui", "tool":"rocket_type", "phase":"queued", "outcome":"refused",
                "native_request":23, "elapsed_ms":21000, "queue_ms":21000, "validation_ms":0,
                "phase_ms":21000, "input_delivered":false, "arguments":"PRIVATE text"
            }),
        ))
        .unwrap();
        let result = aggregate(vec![safe], &CancellationToken::new()).unwrap();
        let entry = &result.entries[0];
        assert!(!entry.incomplete);
        assert_eq!(entry.status, Status::Attention);
        assert_eq!(entry.duration, Some(21000));
        assert!(entry.search.contains("rocket_type queued"));
        assert_eq!(entry.records[0]["native_request"], 23);
        assert_eq!(entry.records[0]["queue_ms"], 21000);
        assert_eq!(entry.records[0]["input_delivered"], false);
        assert!(!entry.summary().contains("PRIVATE"));
        let hidden = project(row(
            2,
            8,
            "mcp_request",
            json!({"outcome":"ok", "tool":"PRIVATE", "phase":"PRIVATE"}),
        ))
        .unwrap();
        assert!(!hidden.to_string().contains("PRIVATE"));
        assert!(project(row(3, 9, "mcp_request", json!({}))).is_err());
    }

    #[test]
    fn inner_failure_and_cancel_override_successful_request_wrapper() {
        let c = CancellationToken::new();
        for (outcome, status) in [
            ("git_failed", Status::Attention),
            ("cancelled", Status::Cancelled),
        ] {
            let rows = vec![
                row(1, 1, "request_start", json!({})),
                row(2, 1, "step_end", json!({"outcome":outcome})),
                row(3, 1, "request_end", json!({"outcome":"ok","elapsed_ms":24})),
            ];
            let s = aggregate(rows, &c).unwrap();
            assert_eq!(s.entries[0].status, status);
        }
        let s = aggregate(
            vec![
                row(1, 1, "request_start", json!({})),
                row(2, 1, "git_end", json!({"outcome":"exited","exit_code":1})),
                row(3, 1, "request_end", json!({"outcome":"ok"})),
            ],
            &c,
        )
        .unwrap();
        assert_eq!(s.entries[0].status, Status::Succeeded);
    }
    #[test]
    fn gaps_missing_ends_and_drops_are_not_success() {
        for rows in [
            vec![
                row(1, 1, "request_start", json!({})),
                row(3, 1, "request_end", json!({"outcome":"ok"})),
            ],
            vec![row(1, 1, "request_end", json!({"outcome":"ok"}))],
            vec![row(1, 1, "request_start", json!({}))],
            vec![
                row(1, 1, "request_start", json!({})),
                row(
                    2,
                    1,
                    "request_end",
                    json!({"outcome":"ok","dropped_before":1}),
                ),
            ],
        ] {
            assert_eq!(
                aggregate(rows, &CancellationToken::new()).unwrap().entries[0].status,
                Status::Incomplete
            );
        }
    }
    #[test]
    fn reader_retains_sessions_rotation_partial_and_filters_payloads() {
        let p = root();
        let old = row(1, 1, "request_start", json!({}));
        write(&p.join("rocket.1.jsonl"), std::slice::from_ref(&old));
        let mut new = row(
            1,
            1,
            "request_start",
            json!({"session":"def-2-1","stdout":"PRIVATE SECRET","path":"素材/PRIVATE","operation":"https://secret.invalid"}),
        );
        write(
            &p.join("rocket.jsonl"),
            &[
                old,
                row(2, 1, "request_end", json!({"outcome":"ok"})),
                new.clone(),
            ],
        );
        {
            use std::io::Write;
            OpenOptions::new()
                .append(true)
                .open(p.join("rocket.jsonl"))
                .unwrap()
                .write_all(b"{partial")
                .unwrap();
        }
        let s = read(&p, &CancellationToken::new()).unwrap();
        assert_eq!(s.records, 3);
        assert_eq!(s.sessions, 2);
        assert_eq!(s.partial, 1);
        assert!(
            s.entries.iter().all(
                |e| !e.summary().contains("PRIVATE") && !e.summary().contains("secret.invalid")
            )
        );
        new["seq"] = json!(3);
        write(&p.join("rocket.jsonl"), &[new]);
        assert!(read(&p, &CancellationToken::new()).unwrap().gaps > 0);
        fs::remove_dir_all(p).unwrap();
    }
    #[test]
    fn reader_rejects_corrupt_oversized_and_cancelled_inputs() {
        let p = root();
        let file = p.join("rocket.jsonl");
        fs::write(&file, "invalid\n").unwrap();
        assert!(read(&p, &CancellationToken::new()).is_err());
        fs::write(&file, vec![b'a'; 16385]).unwrap();
        assert!(read(&p, &CancellationToken::new()).is_err());
        fs::write(&file, vec![b'a'; 1024 * 1024 + 1]).unwrap();
        assert!(read(&p, &CancellationToken::new()).is_err());
        let c = CancellationToken::new();
        c.cancel();
        assert!(matches!(read(&p, &c), Err(Error::Cancelled)));
        fs::remove_dir_all(p).unwrap();
    }
    #[test]
    fn search_filter_and_time_are_deterministic() {
        let mut p = Panel {
            snapshot: aggregate(
                vec![
                    row(1, 1, "request_start", json!({})),
                    row(2, 1, "request_end", json!({"outcome":"ok"})),
                ],
                &CancellationToken::new(),
            )
            .unwrap(),
            query: "LFS".into(),
            ..Default::default()
        };
        assert_eq!(p.matching("abc-1-1"), vec![0]);
        p.filter = Filter::Attention;
        assert!(p.matching("abc-1-1").is_empty());
        p.filter = Filter::All;
        p.current_only = true;
        assert!(p.matching("def-2-1").is_empty());
        assert_eq!(timestamp(0), "1970-01-01 00:00:00 UTC");
        assert_eq!(timestamp(1792395080000), "2026-10-19 07:31:20 UTC");
    }
    #[test]
    fn cancelled_worker_cannot_replace_snapshot_and_corrupt_refresh_keeps_previous_data() {
        let root = root();
        let file = root.join("rocket.jsonl");
        write(
            &file,
            &[
                row(1, 1, "request_start", json!({})),
                row(2, 1, "request_end", json!({"outcome":"ok"})),
            ],
        );
        let retained = read(&root, &CancellationToken::new()).unwrap();
        let (release, gate) = std::sync::mpsc::channel();
        let (started, ready) = std::sync::mpsc::channel();
        let task = ReadTask::spawn(move |_| {
            started.send(()).unwrap();
            gate.recv().unwrap();
            Ok(Snapshot::default())
        })
        .unwrap();
        ready.recv_timeout(Duration::from_secs(5)).unwrap();
        let mut panel = Panel {
            open: true,
            snapshot: retained,
            task: Some(task),
            ..Default::default()
        };
        panel.cancel();
        release.send(()).unwrap();
        let ctx = Context::default();
        panel.poll(root.clone(), &ctx);
        assert_eq!(panel.snapshot.entries.len(), 1);
        assert!(!panel.loading());
        assert!(!panel.automatic);
        fs::write(&file, "malformed\n").unwrap();
        panel.refresh();
        panel.poll(root.clone(), &ctx);
        let deadline = Instant::now() + Duration::from_secs(5);
        while panel.loading() {
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
            panel.poll(root.clone(), &ctx);
        }
        assert!(panel.error.as_ref().unwrap().contains("damaged"));
        assert_eq!(panel.snapshot.entries.len(), 1);
        panel.close();
        assert!(!panel.loading());
        assert_eq!(panel.snapshot.entries.len(), 1);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn reader_rejects_conflicting_sequences_and_missing_completion_outcomes() {
        let p = root();
        write(
            &p.join("rocket.jsonl"),
            &[row(1, 1, "request_end", json!({}))],
        );
        assert!(
            read(&p, &CancellationToken::new())
                .unwrap_err()
                .to_string()
                .contains("damaged")
        );
        write(
            &p.join("rocket.jsonl"),
            &[row(1, 1, "request_start", json!({}))],
        );
        write(
            &p.join("rocket.1.jsonl"),
            &[row(1, 1, "request_start", json!({"operation":"commit"}))],
        );
        assert!(
            read(&p, &CancellationToken::new())
                .unwrap_err()
                .to_string()
                .contains("Conflicting")
        );
        fs::remove_dir_all(p).unwrap();
    }
    fn failed_fetch() -> Vec<Value> {
        vec![
            row(1, 1, "request_start", json!({"id":1,"operation":"fetch"})),
            row(2, 1, "step_start", json!({"id":2,"operation":"discover","step":"verify_git"})),
            row(3, 1, "git_start", json!({"id":3,"operation":"--version","kind":"read"})),
            row(4, 1, "git_end", json!({"id":3,"operation":"--version","elapsed_ms":70,"outcome":"exited","exit_code":0})),
            row(5, 1, "step_end", json!({"id":2,"operation":"discover","step":"verify_git","elapsed_ms":72,"outcome":"ok"})),
            row(6, 1, "step_start", json!({"id":4,"operation":"fetch","step":"validate_selection"})),
            // A key that isn't set: a normal answer, not a failure.
            row(7, 1, "git_start", json!({"id":6,"operation":"config","kind":"read"})),
            row(8, 1, "git_end", json!({"id":6,"operation":"config","elapsed_ms":90,"outcome":"exited","exit_code":1})),
            row(9, 1, "step_end", json!({"id":4,"operation":"fetch","step":"validate_selection","elapsed_ms":96,"outcome":"ok"})),
            row(10, 1, "git_start", json!({"id":5,"operation":"fetch","kind":"read"})),
            row(11, 1, "git_end", json!({"id":5,"operation":"fetch","elapsed_ms":2140,"outcome":"exited","exit_code":128})),
            row(12, 1, "request_end", json!({"id":1,"operation":"fetch","outcome":"git_failed","failure_code":"authentication_rejected","exit_code":128,"elapsed_ms":2310})),
        ]
    }
    #[test]
    fn start_and_end_records_become_one_step_with_setup_marked() {
        let rows = step_rows(&failed_fetch());
        let summary: Vec<(usize, &str, bool, StepState, Option<i64>)> = rows
            .iter()
            .map(|r| (r.depth, r.name.as_str(), r.setup, r.state, r.exit))
            .collect();
        assert_eq!(
            summary,
            [
                (0, "Verify git", true, StepState::Done, None),
                (1, "git --version", true, StepState::Done, None),
                (0, "Validate selection", false, StepState::Done, None),
                (1, "git config", false, StepState::Done, None),
                (0, "git fetch", false, StepState::Failed, Some(128)),
            ]
        );
        assert_eq!(rows[4].elapsed, Some(2140));
    }
    #[test]
    fn commands_running_side_by_side_each_get_their_own_end() {
        let rows = step_rows(&[
            row(1, 1, "git_start", json!({"id":2,"operation":"show-ref"})),
            row(2, 1, "git_start", json!({"id":3,"operation":"rev-parse"})),
            row(3, 1, "git_end", json!({"id":2,"operation":"show-ref","elapsed_ms":5,"outcome":"exited","exit_code":2})),
            row(4, 1, "git_end", json!({"id":3,"operation":"rev-parse","elapsed_ms":7,"outcome":"exited","exit_code":0})),
        ]);
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|r| r.depth == 0 && r.state == StepState::Done));
        assert_eq!((rows[0].elapsed, rows[1].elapsed), (Some(5), Some(7)));
    }
    #[test]
    fn a_failure_with_no_failure_word_in_its_outcome_still_fails() {
        for outcome in ["timed_out", "output_limit", "repository_busy"] {
            let records = [row(1, 1, "git_start", json!({"id":2,"operation":"fetch"})), row(2, 1, "git_end", json!({"id":2,"operation":"fetch","outcome":outcome}))];
            assert_eq!(step_rows(&records)[0].state, StepState::Failed, "{outcome}");
            let problem = problem(Status::Attention, &records).unwrap();
            assert_eq!(problem.place.as_deref(), Some("at git fetch"), "{outcome}");
        }
    }
    #[test]
    fn a_failed_setup_step_is_not_folded_and_a_lost_start_still_shows() {
        let rows = step_rows(&[
            row(1, 1, "step_start", json!({"id":2,"step":"verify_git"})),
            row(2, 1, "step_end", json!({"id":2,"step":"verify_git","outcome":"timed_out"})),
            row(3, 1, "step_end", json!({"id":9,"step":"read_tags","elapsed_ms":5,"outcome":"ok"})),
            row(4, 1, "step_start", json!({"id":10,"step":"still_running"})),
        ]);
        assert!(rows.iter().all(|r| !r.setup), "a failure stays in the open");
        assert_eq!(rows[1].name, "Read tags");
        assert_eq!(rows[2].state, StepState::Running);
    }
    #[test]
    fn a_problem_uses_rockets_own_words_and_says_where_it_stopped() {
        let snapshot = aggregate(failed_fetch(), &CancellationToken::new()).unwrap();
        let e = &snapshot.entries[0];
        let problem = e.problem.as_ref().unwrap();
        let text = rocket_git::failure_text("authentication_rejected").unwrap();
        assert_eq!(problem.headline, text.summary.unwrap());
        assert_eq!(problem.short, "The server rejected authentication.");
        assert_eq!(problem.advice, Some(text.guidance));
        assert_eq!(problem.chips, ["authentication_rejected"]);
        assert_eq!(problem.place.as_deref(), Some("at git fetch · exit 128"));
        assert!(e.search.contains("rejected authentication"), "what shows is searchable");

        // Rocket refused it before Git ran, so the words must not blame Git.
        let refused = vec![
            row(1, 2, "request_start", json!({"operation":"tag-preview"})),
            row(2, 2, "step_end", json!({"id":3,"operation":"tag-preview","step":"review_create_tag","outcome":"tag_refused","failure_code":"tag_invalid_message","level":"warn"})),
            row(3, 2, "request_end", json!({"operation":"tag-preview","outcome":"tag_refused","failure_code":"tag_invalid_message"})),
        ];
        let snapshot = aggregate(refused, &CancellationToken::new()).unwrap();
        let e = &snapshot.entries[0];
        let problem = e.problem.as_ref().unwrap();
        assert_eq!(e.name, "Review tag");
        assert_eq!(problem.headline, rocket_git::failure_text("tag_invalid_message").unwrap().guidance);
        assert_eq!(problem.advice, None);
        assert_eq!(problem.place.as_deref(), Some("at Review create tag"));

        let unknown = vec![
            row(1, 3, "request_start", json!({})),
            row(2, 3, "request_end", json!({"outcome":"ok","failure_code":"some_new_failure"})),
        ];
        let snapshot = aggregate(unknown, &CancellationToken::new()).unwrap();
        assert_eq!(snapshot.entries[0].problem.as_ref().unwrap().headline, "Some new failure.");
    }
    #[test]
    fn only_reads_that_went_fine_are_left_out_and_a_search_finds_them_anyway() {
        let read = |seq, request, op, outcome| {
            vec![
                row(seq, request, "request_start", json!({"operation":op})),
                row(seq + 1, request, "request_end", json!({"operation":op,"outcome":outcome})),
            ]
        };
        let mut records = read(1, 1, "status", "ok");
        records.extend(read(3, 2, "diff", "git_failed"));
        records.extend(read(5, 3, "push", "ok"));
        records.extend(read(7, 4, "hosting-merge-submit", "ok"));
        records.extend([
            row(9, 5, "request_start", json!({"operation":"status"})),
            row(10, 5, "git_start", json!({"operation":"update-ref","kind":"write","id":5})),
            row(11, 5, "git_end", json!({"operation":"update-ref","id":5,"outcome":"exited","exit_code":0})),
            row(12, 5, "request_end", json!({"operation":"status","outcome":"ok"})),
        ]);
        let projected: Vec<Value> = records.into_iter().map(|r| project(r).unwrap()).collect();
        let mut panel = Panel {
            snapshot: aggregate(projected, &CancellationToken::new()).unwrap(),
            ..Default::default()
        };
        let names = |p: &Panel| -> Vec<String> {
            let mut names: Vec<String> = p.visible("abc-1-1").iter().map(|i| p.snapshot.entries[*i].operation.clone()).collect();
            names.sort();
            names
        };
        // A failed read, an action, a write with no Git, and a read that ran a writing command.
        assert_eq!(names(&panel), ["diff", "hosting-merge-submit", "push", "status"]);
        assert_eq!(panel.counts("abc-1-1")[0], 4);
        assert_eq!(panel.reads("abc-1-1"), 1);
        assert_eq!(panel.matching("abc-1-1").len(), 5, "automation still sees every row");
        panel.query = "status".into();
        assert_eq!(names(&panel), ["status", "status"], "a search looks at reads too");
        panel.query = "Read status".into();
        assert_eq!(names(&panel), ["status", "status"], "and at what the row says");
        panel.query.clear();
        panel.show_reads = true;
        assert_eq!(names(&panel).len(), 5);
    }
    #[test]
    fn a_log_of_only_reads_offers_them() {
        let records = vec![
            row(1, 1, "request_start", json!({"operation":"status"})),
            row(2, 1, "request_end", json!({"operation":"status","outcome":"ok"})),
        ];
        let mut panel = Panel {
            snapshot: aggregate(records, &CancellationToken::new()).unwrap(),
            read_once: true,
            ..Default::default()
        };
        for size in [vec2(1280.0, 800.0), vec2(780.0, 620.0)] {
            let widgets = page(&mut panel, size);
            assert!(widgets.contains_key("activity-show-reads"), "{size:?}");
        }
        panel.show_reads = true;
        assert!(page(&mut panel, vec2(1280.0, 800.0)).contains_key("activity-list"));
    }
    #[test]
    fn choosing_a_row_opens_details_only_in_a_narrow_window() {
        let mut panel = Panel {
            snapshot: aggregate(failed_fetch(), &CancellationToken::new()).unwrap(),
            read_once: true,
            ..Default::default()
        };
        let ctx = Context::default();
        crate::style::install(&ctx);
        let click = |panel: &mut Panel, ctx: &Context, size: Vec2| {
            let mut widgets = HashMap::new();
            let run = |panel: &mut Panel, widgets: &mut HashMap<String, UiControl>, events: Vec<Event>| {
                let mut output = ctx.run_ui(
                    RawInput {
                        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
                        events,
                        ..Default::default()
                    },
                    |ui| panel.ui(ui, widgets, "abc-1-1"),
                );
                output.textures_delta.clear();
            };
            run(panel, &mut widgets, vec![]);
            let at = widgets["activity-row-0"].rect.center();
            let press = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
            run(panel, &mut widgets, vec![Event::PointerMoved(at)]);
            run(panel, &mut widgets, vec![press(true)]);
            run(panel, &mut widgets, vec![press(false)]);
        };
        click(&mut panel, &ctx, vec2(1280.0, 800.0));
        assert!(!panel.details_tab);
        click(&mut panel, &ctx, vec2(780.0, 620.0));
        assert!(panel.details_tab);
    }
    #[test]
    fn days_read_as_words_near_today() {
        let now = 1_792_390_000_000;
        assert_eq!(day(now, now), "Today");
        assert_eq!(day(now - 86_400_000, now), "Yesterday");
        let week = day(now - 3 * 86_400_000, now);
        assert!(week.ends_with("day"), "{week}");
        let old = day(now - 30 * 86_400_000, now);
        assert!(old.starts_with("2026-09"), "{old}");
        assert_eq!(started(now, now).split(" at ").count(), 2);
        assert_eq!(clock(now).len(), 5);
    }
    #[test]
    fn day_numbers_count_from_the_epoch() {
        let date = |year, month, day| rocket_git::LocalTime { year, month, day, hour: 0, minute: 0, second: 0 };
        assert_eq!(day_number(&date(1970, 1, 1)), 0);
        assert_eq!(day_number(&date(1970, 3, 1)), 59);
        assert_eq!(day_number(&date(2000, 3, 1)), 11_017);
        assert_eq!(day_number(&civil(1_792_390_000_000)), 1_792_390_000_000 / 86_400_000);
    }
    /// One frame of the page at `size`, and what it published.
    fn page(panel: &mut Panel, size: Vec2) -> HashMap<String, UiControl> {
        let ctx = Context::default();
        crate::style::install(&ctx);
        let mut widgets = HashMap::new();
        let mut output = ctx.run_ui(
            RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
                ..Default::default()
            },
            |ui| panel.ui(ui, &mut widgets, "abc-1-1"),
        );
        output.textures_delta.clear();
        widgets
    }
    #[test]
    fn nothing_read_yet_shows_one_state_not_an_empty_list() {
        let mut panel = Panel::default();
        let widgets = page(&mut panel, vec2(1280.0, 800.0));
        assert!(widgets.contains_key("activity-cancel"), "reading, with Stop");
        assert!(!widgets.contains_key("activity-clear"), "not \"no match\"");
        panel.error = Some("A record in the log is damaged.".into());
        let widgets = page(&mut panel, vec2(1280.0, 800.0));
        assert!(widgets.contains_key("activity-retry"), "failed, with Try again");
        assert!(!widgets.contains_key("activity-guide"), "no log status without a log");
    }
    #[test]
    fn no_match_clears_and_a_narrow_window_uses_tabs() {
        let mut panel = Panel {
            snapshot: aggregate(failed_fetch(), &CancellationToken::new()).unwrap(),
            read_once: true,
            ..Default::default()
        };
        panel.query = "zzz".into();
        let widgets = page(&mut panel, vec2(1280.0, 800.0));
        assert!(widgets.contains_key("activity-clear"));
        assert!(!widgets.contains_key("activity-list"));
        panel.query.clear();
        let widgets = page(&mut panel, vec2(780.0, 620.0));
        assert!(widgets.contains_key("activity-tab-list") && widgets.contains_key("activity-tab-details"));
        assert!(widgets.contains_key("activity-list") && !widgets.contains_key("activity-details"));
        panel.details_tab = true;
        let widgets = page(&mut panel, vec2(780.0, 620.0));
        assert!(widgets.contains_key("activity-details") && !widgets.contains_key("activity-list"));
    }
    #[test]
    fn the_page_counts_each_filter_and_publishes_rows_and_steps() {
        let earlier = json!({"session":"def-2-1"});
        let records = vec![
            row(1, 900, "request_start", json!({})),
            row(2, 900, "step_end", json!({"outcome":"git_failed","failure_code":"authentication_rejected"})),
            row(3, 900, "request_end", json!({"outcome":"ok","elapsed_ms":14})),
            row(4, 901, "request_start", json!({})),
            row(5, 901, "request_end", json!({"outcome":"cancelled","elapsed_ms":5})),
            row(6, 902, "request_start", json!({})),
            row(1, 7, "request_start", earlier.clone()),
            row(2, 7, "git_end", json!({"session":"def-2-1","outcome":"exited","exit_code":0,"spawn_us":4000,"execute_us":120000})),
            row(3, 7, "request_end", json!({"session":"def-2-1","outcome":"ok","elapsed_ms":130})),
        ];
        let mut panel = Panel {
            snapshot: aggregate(records, &CancellationToken::new()).unwrap(),
            ..Default::default()
        };
        // All, Needs attention (a failure and an unfinished request), Cancelled, Succeeded.
        assert_eq!(panel.counts("abc-1-1"), [4, 2, 1, 1]);
        panel.current_only = true;
        assert_eq!(panel.counts("abc-1-1"), [3, 2, 1, 0]);
        panel.current_only = false;
        panel.query = "authentication".into();
        assert_eq!(panel.counts("abc-1-1"), [1, 1, 0, 0], "search narrows the counts");
        panel.query.clear();

        let ctx = Context::default();
        crate::style::install(&ctx);
        let mut widgets = HashMap::new();
        let mut output = ctx.run_ui(
            RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 800.0))),
                ..Default::default()
            },
            |ui| panel.ui(ui, &mut widgets, "abc-1-1"),
        );
        output.textures_delta.clear();
        for id in [
            "activity-close",
            "activity-refresh",
            "activity-auto",
            "activity-search",
            "activity-all",
            "activity-errors",
            "activity-cancelled",
            "activity-succeeded",
            "activity-current",
            "activity-reads",
            "activity-copy",
            "activity-view-steps",
            "activity-view-records",
            "activity-list",
            "activity-details",
        ] {
            assert!(widgets.contains_key(id), "the page shows {id}");
        }
        let rows = widgets.keys().filter(|k| k.starts_with("activity-row-")).count();
        assert_eq!(rows, 4, "every operation has a row");
        assert!(panel.selected.is_some(), "the newest operation is selected");
    }
}
