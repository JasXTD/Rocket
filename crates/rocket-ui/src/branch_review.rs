//! Read-only AI review of a branch. Rocket reads the commits on Target since it split
//! from Base by itself and shows exactly what would go out; only Send gives it to Claude
//! Code or Codex on this computer. Nothing is applied and replies are never saved.
use crate::{
    ai_conflict::{elapsed, field, fix, found, link, pages_size},
    app::*,
    backend::{Backend, user_error},
    commit_draft::{ProgramCheck, Settings, check_program},
    compare::{RefField, kinds, letter, message, ref_field, rename_card, sha, swap_button},
    conflicts::pane_header,
    diff::{FilePatch, Kind as LineKind, Patch},
    folder_browser::PickTarget,
    pages,
    preferences::{card_foot, card_frame, card_header, choices},
    style::*,
};
use eframe::egui::*;
use rocket_git::*;
use serde_json::{Value, json};
use std::{
    collections::{HashMap, VecDeque},
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

/// The Send for review column of a wide page.
const SIDE: f32 = 340.0;
const GAP: f32 = 16.0;
/// Below this width the column goes above the changes.
const WIDE: f32 = 960.0;
/// The file list beside the diff.
const FILES: f32 = 270.0;
/// Typing waits this long for the next key before Rocket reads the new name.
const SETTLE: Duration = Duration::from_millis(700);
/// Where a diff line's text starts, after both line numbers and the +/- mark.
const TEXT_X: f32 = 104.0;
/// A read this quick keeps the shown changes instead of flashing the spinner.
const QUICK: Duration = Duration::from_millis(250);

pub enum ReviewIntent {
    Open,
    /// Read the changes the fields name.
    Read,
    Swap,
    Send,
    /// Stop the running read or review.
    Cancel,
    /// Choose the reviewing CLI's program.
    Browse,
}

/// Which field holds a name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Input {
    Base,
    Target,
}

/// What of a changed file is sent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sent {
    /// Its whole diff: this file of the sent patch.
    Diff(usize),
    /// Only the start of its diff: not one whole file fit.
    Cut(usize),
    /// A binary file: named, its contents never sent.
    Binary,
    /// Left out to fit: only its name.
    Named,
}
impl Sent {
    fn code(self) -> &'static str {
        match self {
            Self::Diff(_) => "diff",
            Self::Cut(_) => "cut",
            Self::Binary => "binary",
            Self::Named => "named",
        }
    }
}

/// One changed file and what of it is sent.
pub struct Row {
    pub file: ComparedFile,
    /// The path as shown, which also finds it in the sent diff.
    pub label: String,
    pub sent: Sent,
}

/// A read: the preview, its files, and the sent diff parsed for display.
pub struct Changes {
    pub preview: BranchReviewPreview,
    pub rows: Vec<Row>,
    patch: Patch,
    /// Each part of the sent diff without Git's headers: indices into its lines.
    shown: Vec<Vec<usize>>,
    /// Each part's longest shown line, in columns.
    longest: Vec<usize>,
    /// The payload's lines, as byte ranges, and the longest in columns.
    exact: Vec<std::ops::Range<usize>>,
    exact_longest: usize,
}
impl Changes {
    fn new(preview: BranchReviewPreview) -> Self {
        let patch = Patch::parse(preview.diff().as_bytes());
        let rows = rows(&preview.files, &patch, preview.scope.shortened());
        let shown: Vec<Vec<usize>> = patch
            .files
            .iter()
            .map(|f| (0..f.lines.len()).filter(|&i| f.lines[i].kind != LineKind::Header).collect())
            .collect();
        let longest = patch
            .files
            .iter()
            .zip(&shown)
            .map(|(f, rows)| rows.iter().map(|&i| columns(&f.lines[i].text)).max().unwrap_or(0))
            .collect();
        let payload = preview.payload();
        let mut exact = Vec::new();
        let mut start = 0;
        for (i, _) in payload.match_indices('\n') {
            exact.push(start..i);
            start = i + 1;
        }
        if start < payload.len() {
            exact.push(start..payload.len());
        }
        let exact_longest = exact.iter().map(|r| columns(&payload[r.clone()])).max().unwrap_or(0);
        Self { preview, rows, patch, shown, longest, exact, exact_longest }
    }
}

/// Columns a line takes in a monospace font: wide characters count twice.
fn columns(text: &str) -> usize {
    text.chars().map(|c| if c == '\t' { 4 } else if c.is_ascii() { 1 } else { 2 }).sum()
}

/// Binds each changed file to its part of the sent diff by its shown path; equal paths
/// go in order. A file with no part was left out to fit.
fn rows(files: &[ComparedFile], patch: &Patch, shortened: bool) -> Vec<Row> {
    let mut parts: HashMap<&str, VecDeque<usize>> = HashMap::new();
    for (i, f) in patch.files.iter().enumerate() {
        parts.entry(f.label.as_str()).or_default().push_back(i);
    }
    files
        .iter()
        .map(|file| {
            let label = crate::text::raw_path_label(file.path.as_bytes());
            let sent = match parts.get_mut(label.as_str()).and_then(VecDeque::pop_front) {
                Some(i) if binary(&patch.files[i]) => Sent::Binary,
                Some(i) if shortened && cut(&patch.files[i]) => Sent::Cut(i),
                Some(i) => Sent::Diff(i),
                None => Sent::Named,
            };
            Row { file: file.clone(), label, sent }
        })
        .collect()
}

fn binary(file: &FilePatch) -> bool {
    file.lines.iter().any(|l| l.kind == LineKind::Header && l.text.starts_with("Binary files "))
}

/// Whether the part stops inside its last hunk: what shortening does to a file too big to
/// send whole.
fn cut(file: &FilePatch) -> bool {
    let Some(start) = file.lines.iter().rposition(|l| l.kind == LineKind::Hunk) else {
        return false;
    };
    let count = |field: &str| field.split_once(',').map_or(Some(1), |(_, n)| n.parse::<u64>().ok());
    let mut fields = file.lines[start].text.strip_prefix("@@ -").unwrap_or("").split_whitespace();
    let old = fields.next().and_then(count);
    let new = fields.next().and_then(|f| f.strip_prefix('+')).and_then(count);
    let (Some(old), Some(new)) = (old, new) else {
        return false;
    };
    let (mut seen_old, mut seen_new) = (0, 0);
    for line in &file.lines[start + 1..] {
        match line.kind {
            LineKind::Context => {
                seen_old += 1;
                seen_new += 1;
            }
            LineKind::Removed => seen_old += 1,
            LineKind::Added => seen_new += 1,
            _ => {}
        }
    }
    seen_old < old || seen_new < new
}

/// What went wrong with a send, in the page's words; the code and the program's own
/// message wait under Details.
pub struct Problem {
    text: String,
    code: String,
    said: Option<String>,
    details: bool,
}

#[derive(Default)]
pub struct State {
    pub base: String,
    pub target: String,
    pub provider: DraftProvider,
    /// Someone chose `provider`; until then each opening takes the first program found.
    chosen: bool,
    /// Defaults were chosen when the page first opened.
    opened: bool,
    pub changes: Option<Arc<Changes>>,
    /// The names that read nothing new: Target has no changes since Base.
    nothing: Option<(String, String)>,
    /// The reply as written, for Copy and automation, and as read.
    pub result: Option<String>,
    pub reply: Option<ReviewReply>,
    /// What the shown reply reviewed: the common ancestor and the target.
    pub result_ids: Option<(ObjectId, ObjectId)>,
    reviewed: Option<(DraftProvider, Instant)>,
    /// The failure code of the last read or send, for automation.
    pub failure: Option<&'static str>,
    problem: Option<Problem>,
    /// Why the last read failed, said under the fields.
    read_problem: Option<String>,
    /// The field whose name resolved to nothing, and that name.
    invalid: Option<(Input, String)>,
    pub notice: Option<String>,
    pub cancelling: bool,
    pub show_findings: bool,
    pub selected: Option<usize>,
    /// The file row and line a finding pointed at, and whether the diff still has to
    /// move there.
    mark: Option<(usize, u32)>,
    jump: bool,
    pub show_exact: bool,
    /// A narrow window shows the file list rather than the diff.
    pub show_files: bool,
    details: bool,
    /// When Base or Target was last typed in.
    edited: Option<Instant>,
    /// The names of the running read, and when it started.
    reading_inputs: Option<(String, String)>,
    reading_since: Option<Instant>,
    /// The IDs a send found moved; once read again the notice says which name moved.
    moved: Option<(ObjectId, ObjectId)>,
    started: Option<Instant>,
    /// Who the running or last review went to.
    asked: DraftProvider,
    reading: Option<ReadTask<Changes>>,
    reviewing: Option<ReadTask<(String, ReviewReply)>>,
    branches_at: Option<u64>,
    branch_choices: Vec<String>,
}
impl State {
    pub fn running(&self) -> bool {
        self.reading.is_some() || self.reviewing.is_some()
    }
    /// Reading the changes.
    pub fn preparing(&self) -> bool {
        self.reading.is_some()
    }
    pub fn sending(&self) -> bool {
        self.reviewing.is_some()
    }
    /// The fields no longer name what the shown changes were read for.
    pub fn changed_inputs(&self) -> bool {
        self.changes.as_ref().is_none_or(|c| {
            c.preview.base_input() != self.base.trim() || c.preview.target_input() != self.target.trim()
        })
    }
    pub fn stop(&self) {
        if let Some(t) = &self.reading {
            t.cancel();
        }
        if let Some(t) = &self.reviewing {
            t.cancel();
        }
    }
    /// Leaving the page stops its work; earlier findings stay with their commit IDs.
    fn leave(&mut self) {
        self.edited = None;
        if self.running() {
            self.stop();
            self.notice = Some(if self.reading.is_some() {
                "Stopped reading. Nothing was sent.".into()
            } else {
                format!("Stopped. {} may already have the preview.", self.asked.label())
            });
            self.reading = None;
            self.reviewing = None;
            self.started = None;
            self.moved = None;
        }
        self.cancelling = false;
    }
    /// The shown reply reviewed the changes shown.
    fn reviewed_now(&self) -> bool {
        match (&self.result_ids, &self.changes) {
            (Some((base, target)), Some(c)) => *base == c.preview.merge_base && *target == c.preview.target,
            _ => false,
        }
    }
    fn can_send(&self, found: bool) -> bool {
        found
            && self.changes.is_some()
            && !self.running()
            && !self.changed_inputs()
            && self.invalid.is_none()
            && self.edited.is_none()
    }
    fn finish_read(&mut self, changes: Changes) {
        if let Some((base, target)) = self.moved.take() {
            let p = &changes.preview;
            let name = if p.target != target || p.base == base { p.target_input() } else { p.base_input() };
            self.notice = Some(format!("{name} moved, so Rocket read it again. Nothing was sent."));
        }
        // The selected file stays selected while it is there.
        let kept = self.selected.and_then(|i| self.changes.as_ref()?.rows.get(i)).map(|r| r.label.clone());
        self.selected = kept
            .and_then(|label| changes.rows.iter().position(|r| r.label == label))
            .or((!changes.rows.is_empty()).then_some(0));
        self.mark = None;
        self.jump = false;
        self.invalid = None;
        self.read_problem = None;
        self.nothing = None;
        self.failure = None;
        self.changes = Some(Arc::new(changes));
    }
    fn read_failed(&mut self, e: Error) {
        if self.moved.take().is_some() {
            self.notice = Some("A name moved, so Rocket read the changes again. Nothing was sent.".into());
        }
        let (base, target) = self.reading_inputs.take().unwrap_or_default();
        self.failure = Some(code(&e));
        match e {
            Error::BranchReview(BranchReviewFailure::BaseNotFound) => self.invalid = Some((Input::Base, base)),
            Error::BranchReview(BranchReviewFailure::TargetNotFound) => self.invalid = Some((Input::Target, target)),
            Error::BranchReview(BranchReviewFailure::NoChanges) => self.nothing = Some((base, target)),
            e => self.read_problem = Some(read_problem(&e, &base, &target)),
        }
    }
    /// What automation reads: never the file names or the findings' text, which the
    /// payload and result carry (and MCP redacts by default).
    pub fn snapshot(&self) -> Value {
        let preview = self.changes.as_ref().map(|c| {
            let p = &c.preview;
            json!({"base":p.base.as_str(),"target":p.target.as_str(),"merge_base":p.merge_base.as_str(),
                "commits":p.commits,"files":p.scope.files,"binary_files":p.scope.binary_files,"omitted_files":p.scope.omitted_files,
                "diff_bytes":p.scope.diff_bytes,"sent_bytes":p.scope.sent_bytes,
                "rows":c.rows.iter().map(|r|json!({"change":letter(r.file.change).to_string(),"sent":r.sent.code()})).collect::<Vec<_>>()})
        });
        let findings = match &self.reply {
            Some(ReviewReply::Findings(f)) => json!(f.findings.len()),
            _ => Value::Null,
        };
        json!({"base":self.base,"target":self.target,"provider":self.provider.code(),
            "running":self.running(),"preparing":self.preparing(),"sending":self.sending(),"cancelling":self.cancelling,
            "waiting":self.edited.is_some(),"changed_inputs":self.changed_inputs(),"failure":self.failure,
            "error":self.problem.as_ref().map(|p|p.text.as_str()).or(self.read_problem.as_deref()),"notice":self.notice,
            "invalid":self.invalid.as_ref().map(|(w,_)|if *w==Input::Base {"base"} else {"target"}),"nothing":self.nothing.is_some(),
            "result":self.result,"result_ids":self.result_ids.as_ref().map(|(a,b)|json!([a.as_str(),b.as_str()])),
            "reviewed_now":self.reviewed_now(),"findings":findings,"plain":matches!(self.reply,Some(ReviewReply::Text(_))),
            "tab":if self.show_findings {"findings"} else {"changes"},"selected":self.selected,"mark":self.mark.map(|(_,l)|l),
            "show_exact":self.show_exact,"show_files":self.show_files,
            "payload":self.changes.as_ref().map(|c|c.preview.payload()),"preview":preview})
    }
}

fn code(e: &Error) -> &'static str {
    match e {
        Error::BranchReview(f) => f.code(),
        Error::CommitDraft { failure, .. } => failure.code(),
        Error::Cancelled => "cancelled",
        Error::Git { .. } => "git_exit",
        Error::OutputLimit { .. } => "output_limit",
        _ => "review_failed",
    }
}

/// A read that failed for another reason than a name or no changes, said under the fields.
fn read_problem(e: &Error, base: &str, target: &str) -> String {
    match e {
        Error::BranchReview(BranchReviewFailure::NoUniqueBase) => {
            format!("{target} and {base} have no single common ancestor. Choose another base.")
        }
        Error::BranchReview(BranchReviewFailure::TooLarge) => {
            format!("The changes on {target} are over 16 MiB, too big to read. Choose a shorter range.")
        }
        Error::BranchReview(f) => f.guidance().to_owned(),
        Error::Git { .. } => "Rocket couldn't read these changes. Check that both commits are in this repository.".into(),
        e => user_error(e),
    }
}

/// A send's failure in the page's words; setup failures keep the shared guidance.
fn problem(e: &Error) -> Problem {
    let plain = |text: String, code: &str, said: Option<String>| Problem { text, code: code.to_owned(), details: said.is_some(), said };
    match e {
        Error::CommitDraft { provider, failure, detail } => {
            let name = provider.label();
            let text = match (failure, provider) {
                (DraftFailure::SignInRequired, DraftProvider::Claude) => {
                    "Claude Code isn't signed in. Run claude in a terminal, sign in, then send again.".into()
                }
                (DraftFailure::SignInRequired, DraftProvider::Codex) => {
                    "Codex isn't signed in. Sign in to the Codex app, or run codex login in a terminal, then send again.".into()
                }
                (DraftFailure::ClaudeFailed | DraftFailure::CodexFailed, _) => format!(
                    "{name} didn't return a review. Run {} in a terminal to check sign-in and usage limits, then try again.",
                    provider.code()
                ),
                (DraftFailure::TimedOut, _) => format!(
                    "{name} didn't answer within {} minutes and was stopped. It may already have the preview.",
                    DRAFT_TIMEOUT.as_secs() / 60
                ),
                (DraftFailure::InvalidReply, _) => format!("{name}'s reply couldn't be used. Try sending again."),
                _ => failure.guidance(*provider).to_owned(),
            };
            let details = matches!(
                failure,
                DraftFailure::SignInRequired | DraftFailure::ClaudeFailed | DraftFailure::CodexFailed | DraftFailure::InvalidReply
            );
            Problem { details, ..plain(text, failure.code(), detail.clone()) }
        }
        Error::BranchReview(f) => plain(f.guidance().to_owned(), f.code(), None),
        e => plain(user_error(e), "review_failed", None),
    }
}

/// Base: main or master. Target: the current branch, unless it is the base or there is
/// none, when the page asks for one instead of comparing a branch with itself.
fn defaults(branches: &[Branch]) -> (String, String) {
    let local = |name: &str| {
        branches
            .iter()
            .any(|b| b.kind == BranchKind::Local && b.reference == format!("refs/heads/{name}").as_bytes())
    };
    let base = ["main", "master"].into_iter().find(|n| local(n)).unwrap_or_default().to_owned();
    let current = branches
        .iter()
        .find(|b| b.is_head && b.kind == BranchKind::Local)
        .map(|b| short_ref(&b.reference));
    (base.clone(), current.filter(|c| *c != base).unwrap_or_default())
}

/// The branch list: local branches, then remote ones.
fn branch_names(branches: &[Branch]) -> Vec<String> {
    let mut names: Vec<String> = branches
        .iter()
        .filter(|b| b.kind == BranchKind::Local)
        .map(|b| short_ref(&b.reference))
        .collect();
    names.extend(
        branches
            .iter()
            .filter(|b| b.kind == BranchKind::Remote && b.symbolic_target.is_none())
            .map(|b| short_ref(&b.reference)),
    );
    names
}

/// The first CLI found reviews until someone chooses.
fn first_found(found: impl Fn(DraftProvider) -> bool) -> DraftProvider {
    if !found(DraftProvider::Claude) && found(DraftProvider::Codex) {
        DraftProvider::Codex
    } else {
        DraftProvider::Claude
    }
}

/// Starts reading the changes the fields name, in place of a read still running.
fn start_read(s: &mut State, ctx: &Context, backend: Option<Backend>, repo: Option<Repository>) {
    let (Some(backend), Some(repo)) = (backend, repo) else {
        return;
    };
    if let Some(task) = s.reading.take() {
        task.cancel();
    }
    s.edited = None;
    s.invalid = None;
    s.read_problem = None;
    s.nothing = None;
    s.cancelling = false;
    if s.moved.is_none() {
        s.notice = None;
    }
    let (base, target) = (s.base.trim().to_owned(), s.target.trim().to_owned());
    if base.is_empty() || target.is_empty() {
        return;
    }
    s.failure = None;
    s.reading_inputs = Some((base.clone(), target.clone()));
    match crate::read_task::spawn(ctx, move |cancel| {
        backend.log.request("branch-review-preview", |log| {
            let preview = repo.with_log(log.clone()).prepare_branch_review(&base, &target, &cancel)?;
            cancel.check()?;
            Ok(Changes::new(preview))
        })
    }) {
        Ok(task) => {
            s.reading = Some(task);
            s.reading_since = Some(Instant::now());
        }
        Err(e) => s.read_failed(e),
    }
}

impl Rocket {
    pub(crate) fn branch_review_intent(&mut self, intent: ReviewIntent) {
        if matches!(intent, ReviewIntent::Open) {
            self.activity_history.close();
            self.close_browsing_page();
        }
        if matches!(intent, ReviewIntent::Browse) {
            if let Some(provider) = self.tab().map(|t| t.branch_review.provider) {
                let current = self.drafting.program_setting_of(provider).to_owned();
                self.browse(PickTarget::AiProgram(provider), &current);
            }
            return;
        }
        let backend = self.backend.clone();
        let Some(tab) = self.tabs.get_mut(self.active) else {
            return;
        };
        let repo = tab.snapshot.as_ref().map(|snap| snap.repository.clone());
        let s = &mut tab.branch_review;
        match intent {
            ReviewIntent::Open => {
                tab.view = View::BranchReview;
                if !s.opened {
                    s.opened = true;
                    if let Some(snap) = &tab.snapshot {
                        (s.base, s.target) = defaults(&snap.branches);
                    }
                }
                if !s.chosen {
                    let drafting = &self.drafting;
                    s.provider = first_found(|p| p.executable(drafting.request_for(p).executable.as_deref()).is_ok());
                }
                // Read when what is shown isn't for these names.
                if !s.running() && s.changed_inputs() {
                    start_read(s, &self.context, backend, repo);
                }
            }
            ReviewIntent::Read if !s.sending() => start_read(s, &self.context, backend, repo),
            ReviewIntent::Swap if !s.sending() => {
                std::mem::swap(&mut s.base, &mut s.target);
                start_read(s, &self.context, backend, repo);
            }
            ReviewIntent::Cancel => {
                if s.running() {
                    s.stop();
                    s.cancelling = true;
                    if let Some(backend) = backend {
                        let _ = backend.log.request("branch-review-cancel", |_| Ok(()));
                    }
                }
            }
            ReviewIntent::Send => {
                let (Some(backend), Some(repo), Some(changes)) = (backend, repo, s.changes.clone()) else {
                    return;
                };
                if s.running() || s.changed_inputs() {
                    return;
                }
                let provider = s.provider;
                let request = self.drafting.request_for(provider);
                s.problem = None;
                s.notice = None;
                s.failure = None;
                s.details = false;
                s.cancelling = false;
                match crate::read_task::spawn(&self.context, move |cancel| {
                    backend.log.request("branch-review", |log| {
                        let text = repo.with_log(log.clone()).review_branch(&changes.preview, &request, &cancel)?;
                        let reply = ReviewReply::read(&text);
                        Ok((text, reply))
                    })
                }) {
                    Ok(task) => {
                        s.reviewing = Some(task);
                        s.started = Some(Instant::now());
                        s.asked = provider;
                    }
                    Err(e) => {
                        s.failure = Some(code(&e));
                        s.problem = Some(problem(&e));
                    }
                }
            }
            _ => {}
        }
    }

    pub(crate) fn poll_branch_reviews(&mut self, ctx: &Context) {
        for tab in &mut self.tabs {
            let s = &mut tab.branch_review;
            if tab.view != View::BranchReview {
                s.leave();
                continue;
            }
            if let Some(result) = s.reading.as_mut().and_then(ReadTask::try_finish) {
                s.reading = None;
                let stopped = std::mem::take(&mut s.cancelling);
                match result {
                    Ok(changes) => {
                        s.reading_inputs = None;
                        s.finish_read(changes);
                    }
                    Err(Error::Cancelled) => {
                        s.reading_inputs = None;
                        s.moved = None;
                        if stopped {
                            s.notice = Some("Stopped reading. Nothing was sent.".into());
                        }
                    }
                    Err(e) => s.read_failed(e),
                }
                ctx.request_repaint();
            }
            let mut again = false;
            if let Some(result) = s.reviewing.as_mut().and_then(ReadTask::try_finish) {
                s.reviewing = None;
                s.started = None;
                let stopped = std::mem::take(&mut s.cancelling);
                match result {
                    Ok(_) | Err(Error::Cancelled) if stopped => {
                        s.notice = Some(format!("Stopped. {} may already have the preview.", s.asked.label()));
                    }
                    Ok((text, reply)) => {
                        s.result = Some(text);
                        s.reply = Some(reply);
                        s.result_ids = s.changes.as_ref().map(|c| (c.preview.merge_base.clone(), c.preview.target.clone()));
                        s.reviewed = Some((s.asked, Instant::now()));
                        s.show_findings = true;
                        s.show_exact = false;
                    }
                    Err(Error::Cancelled) => {
                        s.notice = Some(format!("Stopped. {} may already have the preview.", s.asked.label()));
                    }
                    Err(Error::BranchReview(BranchReviewFailure::Changed)) => {
                        // A name moved since the read: read again, and say so then.
                        s.failure = Some(BranchReviewFailure::Changed.code());
                        s.moved = s.changes.as_ref().map(|c| (c.preview.base.clone(), c.preview.target.clone()));
                        again = true;
                    }
                    Err(e) => {
                        s.failure = Some(code(&e));
                        s.problem = Some(problem(&e));
                    }
                }
                ctx.request_repaint();
            }
            if again {
                let repo = tab.snapshot.as_ref().map(|snap| snap.repository.clone());
                start_read(s, ctx, self.backend.clone(), repo);
            }
            if s.sending() {
                // The card's timer.
                ctx.request_repaint_after(Duration::from_millis(250));
            }
        }
    }
}

fn intent(intents: &mut Vec<Intent>, intent: ReviewIntent) {
    intents.push(Intent::BranchReview(intent));
}

pub(crate) fn view(
    ui: &mut Ui,
    tab: &mut Tab,
    settings: &Settings,
    intents: &mut Vec<Intent>,
    widgets: &mut HashMap<String, UiControl>,
) {
    ui.set_clip_rect(ui.clip_rect().intersect(ui.available_rect_before_wrap()));
    let id = tab.id;
    let s = &mut tab.branch_review;
    if s.branches_at != Some(tab.refresh_count) {
        s.branch_choices = tab.snapshot.as_ref().map(|snap| branch_names(&snap.branches)).unwrap_or_default();
        s.branches_at = Some(tab.refresh_count);
    }
    // A typed name is read once typing settles.
    if let Some(at) = s.edited {
        let waited = at.elapsed();
        if waited >= SETTLE {
            s.edited = None;
            intent(intents, ReviewIntent::Read);
        } else {
            ui.ctx().request_repaint_after(SETTLE - waited);
        }
    }
    let check = check_program(ui.ctx(), settings, s.provider);
    let found = matches!(check, ProgramCheck::Found(_));
    let wide = ui.available_width() - 44.0 >= WIDE;
    header(ui, s, found, intents, widgets);
    bar(ui, s, intents, widgets);
    status(ui, s, wide, widgets);
    let viewport = ui.available_height();
    let area = ScrollArea::vertical()
        .id_salt(("review-page", id))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            Frame::new()
                .inner_margin(Margin { left: 24, right: 20, top: 0, bottom: 16 })
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    let width = ui.available_width();
                    if wide {
                        let height = (viewport - 16.0).max(360.0);
                        ui.horizontal_top(|ui| {
                            ui.spacing_mut().item_spacing.x = GAP;
                            ui.allocate_ui_with_layout(vec2(width - SIDE - GAP, height), Layout::top_down(Align::Min), |ui| {
                                ui.spacing_mut().item_spacing.y = 0.0;
                                panel(ui, s, false, height, intents, widgets);
                            });
                            ui.allocate_ui_with_layout(vec2(SIDE, height), Layout::top_down(Align::Min), |ui| {
                                ui.spacing_mut().item_spacing.y = 0.0;
                                notes(ui, s, widgets);
                                card(ui, s, &check, intents, widgets);
                            });
                        });
                    } else {
                        let top = ui.cursor().top();
                        notes(ui, s, widgets);
                        stacked_card(ui, s, &check, intents, widgets);
                        let used = ui.cursor().top() - top;
                        panel(ui, s, true, (viewport - used - 16.0).max(300.0), intents, widgets);
                    }
                });
        });
    pages::region(widgets, "review-page-scroll", area.inner_rect.intersect(ui.clip_rect()), true);
}

/// The title, and the main button, which keeps its place: Send, then Stop while
/// reviewing, then Send again once the shown changes have findings.
fn header(ui: &mut Ui, s: &State, found: bool, intents: &mut Vec<Intent>, widgets: &mut HashMap<String, UiControl>) {
    pages::header(ui, Icon::Sparkle, "AI review", "Read only · nothing is sent until you press Send", |ui| {
        if s.sending() {
            let r = ui.add_enabled(!s.cancelling, Button::new(if s.cancelling { "Stopping" } else { "Stop" }));
            pages::control(widgets, "review-cancel", &r, ui.clip_rect());
            if r.clicked() {
                intent(intents, ReviewIntent::Cancel);
            }
            return;
        }
        let ready = s.can_send(found);
        let r = if s.reviewed_now() {
            ui.add_enabled(ready, Button::new("Send again"))
        } else {
            pages::primary(ui, ready, &format!("Send to {}", s.provider.label()))
        };
        pages::control(widgets, "review-send", &r, ui.clip_rect());
        if r.clicked() {
            intent(intents, ReviewIntent::Send);
        }
    });
}

/// Base and Target as in Compare, Swap between them, and Read again.
fn bar(ui: &mut Ui, s: &mut State, intents: &mut Vec<Intent>, widgets: &mut HashMap<String, UiControl>) {
    Frame::new()
        .fill(PANEL())
        .stroke(Stroke::new(1.0, CARD_LINE()))
        .corner_radius(10)
        .inner_margin(Margin::symmetric(12, 10))
        .outer_margin(Margin { left: 24, right: 20, top: 0, bottom: 0 })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.add_enabled_ui(!s.sending(), |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    let width = ((ui.available_width() - 2.0 * 30.0 - 3.0 * 8.0) / 2.0).max(110.0);
                    let (base, target) = (s.base.clone(), s.target.clone());
                    let bad = |which: Input, text: &str| {
                        s.invalid.as_ref().is_some_and(|(w, name)| *w == which && name == text.trim())
                    };
                    let (base_bad, target_bad) = (bad(Input::Base, &s.base), bad(Input::Target, &s.target));
                    let mut enter = false;
                    let field = RefField { key: "review-base", label: "BASE", hint: "Branch, tag or commit", invalid: base_bad };
                    let mut picked = ref_field(ui, widgets, field, &mut s.base, &s.branch_choices, width, &mut enter);
                    let swap = swap_button(ui, widgets, "review-swap", true);
                    let field = RefField { key: "review-target", label: "TARGET", hint: "Choose a branch", invalid: target_bad };
                    picked |= ref_field(ui, widgets, field, &mut s.target, &s.branch_choices, width, &mut enter);
                    let filled = !s.base.trim().is_empty() && !s.target.trim().is_empty();
                    let r = ui
                        .add_enabled(filled, Button::new("").min_size(vec2(30.0, 30.0)))
                        .on_hover_text("Read the changes again");
                    r.widget_info(|| WidgetInfo::labeled(WidgetType::Button, filled, "Read the changes again"));
                    icon(ui.painter(), Rect::from_center_size(r.rect.center(), vec2(14.0, 14.0)), Icon::Refresh, if filled { TEXT() } else { FAINT() });
                    pages::control(widgets, "review-read", &r, ui.clip_rect());
                    if swap {
                        intent(intents, ReviewIntent::Swap);
                    } else if picked || enter || r.clicked() {
                        intent(intents, ReviewIntent::Read);
                    } else if s.base != base || s.target != target {
                        s.edited = Some(Instant::now());
                    }
                });
            });
        });
}

/// Under the bar: a name that resolved to nothing, a read that failed, or what was read.
fn status(ui: &mut Ui, s: &State, wide: bool, widgets: &mut HashMap<String, UiControl>) {
    Frame::new()
        .inner_margin(Margin { left: 26, right: 22, top: 9, bottom: 9 })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 2.0;
            if let Some((_, name)) = &s.invalid {
                ui.label(RichText::new(format!("No branch, tag or commit named {name}.")).size(12.5).color(RED()));
                if s.changes.is_some() {
                    ui.label(RichText::new("The last preview stays until a name is found.").size(12.0).color(HINT()));
                }
            } else if let Some(text) = &s.read_problem {
                ui.add(Label::new(RichText::new(text).size(12.5).color(RED())).wrap());
            } else if let Some(c) = s.changes.as_ref().filter(|_| wide) {
                result_line(ui, s, c, widgets);
            } else if wide {
                ui.add_space(20.0);
            }
        });
}

/// What was read: where Target split from Base, its commits and files, and whether
/// anything was sent.
fn result_line(ui: &mut Ui, s: &State, c: &Changes, widgets: &mut HashMap<String, UiControl>) {
    let p = &c.preview;
    let sent = match s.reviewed.filter(|_| s.reviewed_now()) {
        Some((who, at)) => {
            let age = at.elapsed().as_secs();
            let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
            // Keep "… ago" current while the page is idle.
            ui.ctx().request_repaint_after(Duration::from_secs(30));
            format!("Reviewed by {} · {}", who.label(), ago(age, now.saturating_sub(age) as i64))
        }
        None if s.reply.is_some() => "The findings are for other commits".into(),
        None => "Nothing sent yet".into(),
    };
    ui.horizontal(|ui| {
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(RichText::new(sent).size(12.0).color(HINT()));
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                let split = format!("Where {} split from {}", p.target_input(), p.base_input());
                sha(ui, widgets, "review-copy-base", &p.merge_base, &split);
                ui.label(RichText::new("→").color(HINT()));
                sha(ui, widgets, "review-copy-target", &p.target, p.target_input());
                let commits = if p.commits == 1 { "1 commit".to_owned() } else { format!("{} commits", p.commits) };
                ui.label(RichText::new(commits).font(semibold(13.0)));
                let n = p.files.len();
                ui.label(RichText::new(format!("{n} file{}", if n == 1 { "" } else { "s" })).font(semibold(13.0)));
                for (count, word, kind) in kinds(&p.files) {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 5.0;
                        let (dot, _) = ui.allocate_exact_size(vec2(7.0, 7.0), Sense::hover());
                        ui.painter().circle_filled(dot.center(), 3.5, status_color(kind));
                        ui.label(RichText::new(format!("{count} {word}")).size(12.5).color(MUTED()));
                    });
                }
            });
        });
    });
}

/// The changes and the findings in one card; until there is something to show, what is
/// missing.
fn panel(ui: &mut Ui, s: &mut State, narrow: bool, height: f32, intents: &mut Vec<Intent>, widgets: &mut HashMap<String, UiControl>) {
    Frame::new()
        .fill(BG())
        .stroke(Stroke::new(1.0, CARD_LINE()))
        .corner_radius(8)
        .inner_margin(1)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.set_min_height(height - 2.0);
            ui.spacing_mut().item_spacing.y = 0.0;
            let (base, target) = (s.base.trim().to_owned(), s.target.trim().to_owned());
            let quick = s.changes.is_some() && s.reading_since.is_some_and(|at| at.elapsed() < QUICK);
            if s.preparing() && quick {
                ui.ctx().request_repaint_after(QUICK);
            } else if s.preparing() {
                let stopping = s.cancelling;
                pages::loading_with(ui, "Reading the changes", 38.0, |ui| {
                    ui.add_space(8.0);
                    let r = ui.add_enabled(!stopping, Button::new(if stopping { "Stopping" } else { "Stop" }));
                    pages::control(widgets, "review-cancel", &r, ui.clip_rect());
                    if r.clicked() {
                        intent(intents, ReviewIntent::Cancel);
                    }
                });
                return;
            }
            if base.is_empty() || target.is_empty() {
                choose(ui, s, intents, widgets);
                return;
            }
            if s.nothing.as_ref().is_some_and(|(b, t)| *b == base && *t == target) {
                let hint = format!("It has no changes since it split from {base}.");
                message(ui, Some((Icon::Sparkle, BLUE())), &format!("Nothing new on {target}"), &hint, |_| ());
                return;
            }
            let Some(changes) = s.changes.clone() else {
                message(ui, None, "Nothing to show until the changes are read.", "", |_| ());
                return;
            };
            tabs_bar(ui, s, &changes, narrow, widgets);
            let rest = ui.available_height();
            if s.show_findings {
                findings(ui, s, &changes, widgets);
            } else if s.show_exact {
                exact(ui, s, &changes, widgets);
            } else if narrow {
                if s.show_files {
                    file_list(ui, s, &changes, widgets);
                } else {
                    diff_pane(ui, s, &changes, widgets);
                }
            } else {
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    ui.allocate_ui_with_layout(vec2(FILES, rest), Layout::top_down(Align::Min), |ui| {
                        ui.set_min_size(vec2(FILES, rest));
                        file_list(ui, s, &changes, widgets);
                    });
                    let (line, _) = ui.allocate_exact_size(vec2(1.0, rest), Sense::hover());
                    ui.painter().rect_filled(line, 0.0, CARD_LINE());
                    ui.allocate_ui_with_layout(vec2(ui.available_width(), rest), Layout::top_down(Align::Min), |ui| {
                        diff_pane(ui, s, &changes, widgets);
                    });
                });
            }
        });
}

/// Opened on the base itself, or with a field empty: ask for the branch.
fn choose(ui: &mut Ui, s: &mut State, intents: &mut Vec<Intent>, widgets: &mut HashMap<String, UiControl>) {
    let base = s.base.trim().to_owned();
    let hint = if base.is_empty() {
        "Pick a base and a branch above. Branches, tags and commit IDs all work.".to_owned()
    } else {
        format!("Rocket reads its commits since it split from {base}.")
    };
    message(ui, Some((Icon::Sparkle, BLUE())), "Choose a branch to review", &hint, |ui| {
        ui.add_space(6.0);
        let choices: Vec<String> = s.branch_choices.iter().filter(|b| **b != base).cloned().collect();
        // A combo box lays itself out in a row; its own cell keeps it under the message.
        ui.allocate_ui_with_layout(vec2(150.0, 26.0), Layout::left_to_right(Align::Center), |ui| {
            ui.spacing_mut().combo_width = 150.0;
            if let Some(name) = crate::branch_picker::pick(ui, widgets, "review-choose", "Choose branch", &choices, true) {
                if s.target.trim().is_empty() {
                    s.target = name;
                } else {
                    s.base = name;
                }
                intent(intents, ReviewIntent::Read);
            }
        });
    });
}

/// A tab of the card's switch: its name and a count, framed only when selected.
fn tab_button(ui: &mut Ui, label: &str, count: Option<&str>, selected: bool, enabled: bool) -> Response {
    let mark = ui.next_auto_id().with("count");
    let ink = if selected { TEXT() } else if enabled { MUTED() } else { FAINT() };
    let chip = count.map(|c| ui.painter().layout_no_wrap(c.to_owned(), semibold(10.5), ink));
    let text = RichText::new(label).size(12.5).color(ink);
    let button = match &chip {
        Some(g) => Button::new((text, Atom::custom(mark, vec2(g.size().x + 10.0, 16.0)))),
        None => Button::new(text),
    }
    .selected(selected)
    .fill(if selected { SELECTED() } else { Color32::TRANSPARENT })
    .corner_radius(5)
    .min_size(vec2(0.0, 24.0));
    let out = ui
        .add_enabled_ui(enabled, |ui| {
            if !selected {
                let idle = &mut ui.visuals_mut().widgets.inactive;
                idle.weak_bg_fill = Color32::TRANSPARENT;
                idle.bg_fill = Color32::TRANSPARENT;
                idle.bg_stroke.color = Color32::TRANSPARENT;
            }
            button.frame_when_inactive(true).atom_ui(ui)
        })
        .inner;
    if let (Some(galley), Some(rect)) = (chip, out.rect(mark)) {
        let pill = Rect::from_center_size(rect.center(), vec2(galley.size().x + 10.0, 16.0));
        ui.painter().rect_filled(pill, 8.0, if selected { BLUE().gamma_multiply(0.22) } else { HAIRLINE() });
        ui.painter().galley(pill.center() - galley.size() / 2.0, galley, ink);
    }
    out.response
}

/// Changes | Findings, and on the right what the tab offers.
fn tabs_bar(ui: &mut Ui, s: &mut State, c: &Changes, narrow: bool, widgets: &mut HashMap<String, UiControl>) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 44.0), Sense::hover());
    ui.painter().hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0, CARD_LINE()));
    let mut bar = ui.new_child(UiBuilder::new().max_rect(rect.shrink2(vec2(10.0, 0.0))).layout(Layout::right_to_left(Align::Center)));
    bar.spacing_mut().item_spacing.x = 8.0;
    if s.show_findings {
        let text = match &s.reply {
            Some(ReviewReply::Findings(f)) => findings_text(f),
            Some(ReviewReply::Text(t)) => t.clone(),
            None => String::new(),
        };
        let r = pages::tool(&mut bar, Icon::Copy, "Copy findings", !text.is_empty());
        pages::control(widgets, "review-copy", &r, bar.clip_rect());
        if r.clicked() {
            bar.ctx().copy_text(text);
        }
    } else if narrow && !s.show_exact {
        let items = [("Files", s.show_files), ("Diff", !s.show_files)];
        let ids = ["review-files".to_owned(), "review-diff".to_owned()];
        if let Some(i) = choices(&mut bar, widgets, &items, &ids) {
            s.show_files = i == 0;
        }
    } else {
        bar.label(RichText::new(format!("Exactly what {} gets", s.provider.label())).size(12.0).color(HINT()));
    }
    bar.with_layout(Layout::left_to_right(Align::Center), |ui| {
        Frame::new()
            .fill(RAISED())
            .stroke(Stroke::new(1.0, HAIRLINE()))
            .corner_radius(7)
            .inner_margin(2)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    ui.spacing_mut().button_padding = vec2(10.0, 3.0);
                    let total = c.rows.len().to_string();
                    let r = tab_button(ui, "Changes", Some(&total), !s.show_findings, true);
                    pages::control(widgets, "review-tab-changes", &r, ui.clip_rect());
                    if r.clicked() {
                        s.show_findings = false;
                    }
                    let count = match &s.reply {
                        Some(ReviewReply::Findings(f)) => Some(f.findings.len().to_string()),
                        Some(ReviewReply::Text(_)) => Some("·".to_owned()),
                        None => None,
                    };
                    let r = tab_button(ui, "Findings", count.as_deref(), s.show_findings, s.reply.is_some());
                    pages::control(widgets, "review-tab-findings", &r, ui.clip_rect());
                    if r.clicked() {
                        s.show_findings = true;
                    }
                });
            });
    });
}

/// The changed files as the Changes list shows them, with what of each is sent.
fn file_list(ui: &mut Ui, s: &mut State, c: &Changes, widgets: &mut HashMap<String, UiControl>) {
    ui.spacing_mut().item_spacing.y = 0.0;
    let mut picked = None;
    let out = ScrollArea::vertical()
        .id_salt("review-files")
        .auto_shrink([false, false])
        .show_rows(ui, 30.0, c.rows.len(), |ui, range| {
            for i in range {
                let row = &c.rows[i];
                let (rect, r) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click());
                let shown = rect.shrink2(vec2(6.0, 1.0));
                row_background(ui.painter(), shown, s.selected == Some(i), r.hovered());
                status_badge(ui.painter(), pos2(shown.left() + 15.0, shown.center().y), letter(row.file.change));
                let only_named = matches!(row.sent, Sent::Binary | Sent::Named);
                // On the right: the lines sent, or that only the name is.
                let mut right = shown.right() - 8.0;
                let mut paint = |text: String, color: Color32| {
                    let galley = ui.painter().layout_no_wrap(text, FontId::proportional(12.0), color);
                    right -= galley.size().x;
                    ui.painter().galley(pos2(right, shown.center().y - galley.size().y / 2.0), galley, color);
                    right -= 6.0;
                };
                match row.sent {
                    Sent::Diff(part) => {
                        let file = &c.patch.files[part];
                        if file.removed > 0 {
                            paint(format!("−{}", file.removed), RED());
                        }
                        if file.added > 0 {
                            paint(format!("+{}", file.added), GREEN());
                        }
                    }
                    Sent::Cut(_) => paint("start only".into(), AMBER()),
                    Sent::Binary => paint("binary, named only".into(), FAINT()),
                    Sent::Named => paint("named only".into(), FAINT()),
                }
                let (dir, name) = row.label.rsplit_once('/').unwrap_or(("", row.label.as_str()));
                let detail = match &row.file.original_path {
                    Some(old) => format!("← {}", crate::text::raw_path_label(old.as_bytes())),
                    None if dir.is_empty() => String::new(),
                    None => format!("{dir}/"),
                };
                let text = Rect::from_min_max(pos2(shown.left() + 32.0, shown.top()), pos2(right - 4.0, shown.bottom()));
                let ink = if only_named { FAINT() } else { TEXT() };
                let galley = ui.painter().layout_no_wrap(name.to_owned(), FontId::proportional(13.0), ink);
                let width = galley.size().x.min(text.width().max(0.0));
                let top = shown.center().y - galley.size().y / 2.0;
                ui.painter().with_clip_rect(text.intersect(ui.clip_rect())).galley(pos2(text.left(), top), galley, ink);
                if row.file.change == Change::Deleted {
                    ui.painter().hline(text.left()..=text.left() + width, shown.center().y, Stroke::new(1.0, RED().gamma_multiply(0.7)));
                }
                let after = text.left() + width + 8.0;
                if !detail.is_empty() && after < text.right() - 20.0 {
                    let rest = Rect::from_min_max(pos2(after, shown.top()), pos2(text.right(), shown.bottom()));
                    clipped_text(ui, rest, &detail, 12.0, FAINT());
                }
                let what = match row.sent {
                    Sent::Diff(_) => "sent whole",
                    Sent::Cut(_) => "only the start of its diff is sent",
                    Sent::Binary => "binary: only its name is sent",
                    Sent::Named => "left out to fit: only its name is sent",
                };
                let r = r.on_hover_text(format!("{}\n{}, {what}", row.label, crate::compare::change_label(row.file.change)));
                r.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, format!("{}: {what}", row.label)));
                pages::control(widgets, format!("review-file-{i}"), &r, ui.clip_rect());
                if r.clicked() {
                    picked = Some(i);
                }
            }
        });
    pages::region(widgets, "review-files-scroll", out.inner_rect.intersect(ui.clip_rect()), true);
    if let Some(i) = picked {
        s.selected = Some(i);
        s.mark = None;
        s.show_files = false;
    }
}

/// The selected file's part of the diff, exactly as sent.
fn diff_pane(ui: &mut Ui, s: &mut State, c: &Changes, widgets: &mut HashMap<String, UiControl>) {
    let Some(index) = s.selected.filter(|i| *i < c.rows.len()) else {
        message(ui, None, "Select a file to see what is sent.", "", |_| ());
        return;
    };
    let row = &c.rows[index];
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 38.0), Sense::hover());
    let painter = ui.painter();
    painter.hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0, CARD_LINE()));
    let (note, note_color) = match row.sent {
        Sent::Diff(_) => ("As sent", HINT()),
        Sent::Cut(_) => ("Only the start is sent", AMBER()),
        Sent::Binary | Sent::Named => ("Named only", HINT()),
    };
    let note = painter.layout_no_wrap(note.to_owned(), FontId::proportional(12.0), note_color);
    let end = rect.right() - 12.0 - note.size().x;
    painter.galley(pos2(end, rect.center().y - note.size().y / 2.0), note, note_color);
    let mut x = rect.left() + 12.0;
    let room = Rect::from_min_max(rect.min, pos2(end - 8.0, rect.max.y));
    let clipped = painter.with_clip_rect(room.intersect(ui.clip_rect()));
    let path = clipped.layout_no_wrap(row.label.clone(), FontId::monospace(12.5), TEXT());
    let path_width = path.size().x;
    clipped.galley(pos2(x, rect.center().y - path.size().y / 2.0), path, TEXT());
    x += path_width + 10.0;
    if let Sent::Diff(part) | Sent::Cut(part) = row.sent {
        let file = &c.patch.files[part];
        for (text, color) in [(format!("+{}", file.added), GREEN()), (format!("−{}", file.removed), RED())] {
            let galley = clipped.layout_no_wrap(text, FontId::proportional(12.0), color);
            let w = galley.size().x;
            clipped.galley(pos2(x, rect.center().y - galley.size().y / 2.0), galley, color);
            x += w + 6.0;
        }
    }
    match row.sent {
        Sent::Named => {
            let hint = format!("Its changes were left out to fit the {} KiB limit.", DRAFT_DIFF_LIMIT / 1024);
            message(ui, Some((Icon::File, AMBER())), "Only its name is sent", &hint, |_| ());
        }
        Sent::Binary => {
            message(ui, Some((Icon::File, MUTED())), "Binary file", "Only its name is sent. Rocket never sends binary contents.", |_| ());
        }
        Sent::Diff(part) | Sent::Cut(part) => {
            if c.shown[part].is_empty() {
                if row.file.original_path.is_some() {
                    rename_card(ui, &row.file);
                } else {
                    message(ui, None, "No lines changed", "Only the file's mode or type changed.", |_| ());
                }
            } else {
                lines(ui, s, c, index, part, widgets);
            }
        }
    }
}

/// A part's lines in rows like the diff view's; only the visible rows are laid out.
fn lines(ui: &mut Ui, s: &mut State, c: &Changes, row: usize, part: usize, widgets: &mut HashMap<String, UiControl>) {
    let file = &c.patch.files[part];
    let rows = &c.shown[part];
    let font = FontId::monospace(12.5);
    let height = ui.text_style_height(&TextStyle::Monospace) + 5.0;
    let glyph = ui.fonts_mut(|f| f.glyph_width(&font, '0'));
    let width = (TEXT_X + 16.0 + c.longest[part] as f32 * glyph).max(ui.available_width());
    let mark = s.mark.filter(|(r, _)| *r == row).map(|(_, line)| u64::from(line));
    let mut area = ScrollArea::both().id_salt(("review-diff", row)).auto_shrink([false, false]);
    if std::mem::take(&mut s.jump)
        && let Some(at) = mark.and_then(|line| rows.iter().position(|&i| file.lines[i].new == Some(line)))
    {
        // egui clamps an offset only while a scroll bar shows.
        let end = (rows.len() as f32 * height - ui.available_height()).max(0.0);
        area = area.vertical_scroll_offset((at.saturating_sub(4) as f32 * height).min(end));
    }
    let out = area.show_rows(ui, height, rows.len(), |ui, range| {
        for k in range {
            let line = &file.lines[rows[k]];
            let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
            let painter = ui.painter();
            let (fill, marker, marker_color) = match line.kind {
                LineKind::Added => (Some(ADDED_ROW()), "+", GREEN()),
                LineKind::Removed => (Some(REMOVED_ROW()), "-", RED()),
                LineKind::Hunk => (Some(VIEW_RAISED()), "", MUTED()),
                _ => (None, "", MUTED()),
            };
            if let Some(fill) = fill {
                painter.rect_filled(rect, 0.0, fill);
            }
            let y = rect.center().y;
            if line.kind == LineKind::Hunk {
                painter.text(pos2(rect.left() + TEXT_X, y), Align2::LEFT_CENTER, &line.text, font.clone(), MUTED());
                continue;
            }
            if mark.is_some() && line.new == mark {
                painter.rect_filled(Rect::from_min_size(rect.min, vec2(3.0, rect.height())), 0.0, BLUE());
            }
            let number = |n: Option<u64>| n.map_or_else(String::new, |n| n.to_string());
            painter.text(pos2(rect.left() + 40.0, y), Align2::RIGHT_CENTER, number(line.old), FontId::monospace(11.5), FAINT());
            painter.text(pos2(rect.left() + 78.0, y), Align2::RIGHT_CENTER, number(line.new), FontId::monospace(11.5), FAINT());
            painter.text(pos2(rect.left() + 91.0, y), Align2::CENTER_CENTER, marker, font.clone(), marker_color);
            let text = line.text.get(1..).unwrap_or("").replace('\t', "    ");
            painter.text(pos2(rect.left() + TEXT_X, y), Align2::LEFT_CENTER, text, font.clone(), TEXT());
        }
    });
    pages::region(widgets, "review-diff-scroll", out.inner_rect.intersect(ui.clip_rect()), true);
}

/// The payload as text, exactly as the reviewer gets it.
fn exact(ui: &mut Ui, s: &State, c: &Changes, widgets: &mut HashMap<String, UiControl>) {
    let payload = c.preview.payload();
    pane_header(ui, "Exact data", &format!("what {} gets", s.provider.label()), &pages_size(payload.len()), BLUE());
    let font = FontId::monospace(12.0);
    let height = ui.text_style_height(&TextStyle::Monospace) + 3.0;
    let glyph = ui.fonts_mut(|f| f.glyph_width(&font, '0'));
    let width = (24.0 + c.exact_longest as f32 * glyph).max(ui.available_width());
    let out = ScrollArea::both()
        .id_salt("review-exact")
        .auto_shrink([false, false])
        .show_rows(ui, height, c.exact.len(), |ui, range| {
            for i in range {
                let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
                let text = payload[c.exact[i].clone()].replace('\t', "    ");
                ui.painter().text(pos2(rect.left() + 12.0, rect.center().y), Align2::LEFT_CENTER, text, font.clone(), TEXT());
            }
        });
    pages::region(widgets, "review-exact-scroll", out.inner_rect.intersect(ui.clip_rect()), true);
}

/// The row of a file a reviewer named: its exact path, or the one path ending in it.
fn locate(rows: &[Row], file: &str) -> Option<usize> {
    let file = file.trim().trim_matches('`').replace('\\', "/");
    let file = file.trim_start_matches("./");
    let file = file.strip_prefix("a/").or_else(|| file.strip_prefix("b/")).filter(|_| !rows.iter().any(|r| r.label == file)).unwrap_or(file);
    if file.is_empty() {
        return None;
    }
    if let Some(i) = rows.iter().position(|r| r.label == file) {
        return Some(i);
    }
    let suffix = format!("/{file}");
    let mut ends = rows.iter().enumerate().filter(|(_, r)| r.label.ends_with(&suffix));
    match (ends.next(), ends.next()) {
        (Some((i, _)), None) => Some(i),
        _ => None,
    }
}

/// `path:line` in a reply's text, for a file of these changes: the row and the line.
fn place(rows: &[Row], token: &str) -> Option<(usize, u32)> {
    let mut parts = token.rsplitn(3, ':').collect::<Vec<_>>();
    parts.reverse();
    // path:line, or path:line:column.
    let (path, line) = match parts.as_slice() {
        [path, line, column] if column.bytes().all(|b| b.is_ascii_digit()) && !column.is_empty() => (*path, *line),
        [path, line] => (*path, *line),
        [first, path, line] => (&token[..first.len() + 1 + path.len()], *line),
        _ => return None,
    };
    let line = line.parse::<u32>().ok().filter(|n| *n > 0)?;
    Some((locate(rows, path)?, line))
}

/// A paragraph cut into plain text and `path:line` places of these changes.
fn pieces<'a>(rows: &[Row], paragraph: &'a str) -> Vec<(&'a str, Option<(usize, u32)>)> {
    const OPEN: &[char] = &['(', '[', '{', '<', '"', '\'', '`'];
    const CLOSE: &[char] = &[')', ']', '}', '>', '"', '\'', '`', ',', '.', ';', ':', '!', '?'];
    let mut out: Vec<(&str, Option<(usize, u32)>)> = Vec::new();
    let mut plain = 0;
    let mut at = 0;
    for word in paragraph.split(' ') {
        let start = at + (word.len() - word.trim_start_matches(OPEN).len());
        let core = word.trim_start_matches(OPEN).trim_end_matches(CLOSE);
        if !core.is_empty()
            && let Some(found) = place(rows, core)
        {
            if plain < start {
                out.push((&paragraph[plain..start], None));
            }
            out.push((&paragraph[start..start + core.len()], Some(found)));
            plain = start + core.len();
        }
        at += word.len() + 1;
    }
    if plain < paragraph.len() {
        out.push((&paragraph[plain..], None));
    }
    out
}

/// The findings as text, for Copy.
fn findings_text(f: &ReviewFindings) -> String {
    let mut out = String::new();
    if !f.summary.is_empty() {
        out.push_str(&f.summary);
        out.push_str("\n\n");
    }
    for finding in &f.findings {
        let kind = if finding.kind == FindingKind::Bug { "Bug" } else { "Check" };
        out.push_str(&format!("{kind}: {}", finding.title));
        match (finding.file.is_empty(), finding.line) {
            (false, Some(line)) => out.push_str(&format!(" ({}:{line})", finding.file)),
            (false, None) => out.push_str(&format!(" ({})", finding.file)),
            _ => {}
        }
        out.push('\n');
        for (prefix, text) in [("", &finding.why), ("Fix: ", &finding.fix)] {
            if !text.is_empty() {
                out.push_str(&format!("{prefix}{text}\n"));
            }
        }
        out.push('\n');
    }
    if !f.not_checked.is_empty() {
        out.push_str(&format!("Not checked: {}\n", f.not_checked));
    }
    out.trim_end().to_owned()
}

/// The reply: a summary and a card for each finding, or the text as written.
fn findings(ui: &mut Ui, s: &mut State, c: &Changes, widgets: &mut HashMap<String, UiControl>) {
    let Some(reply) = s.reply.as_ref() else {
        return;
    };
    let mut jump: Option<(usize, Option<u32>)> = None;
    let out = ScrollArea::vertical()
        .id_salt("review-findings")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            Frame::new().inner_margin(Margin::symmetric(16, 14)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 10.0;
                match reply {
                    ReviewReply::Findings(f) if f.findings.is_empty() => {
                        ui.add_space(40.0);
                        ui.vertical_centered(|ui| {
                            ui.spacing_mut().item_spacing.y = 8.0;
                            found(ui, "No problems found");
                            ui.label(RichText::new("That is not proof the code is right.").size(12.5).color(HINT()));
                            if !f.summary.is_empty() {
                                ui.add(Label::new(RichText::new(&f.summary).size(12.5).color(MUTED())).wrap());
                            }
                        });
                    }
                    ReviewReply::Findings(f) => {
                        for_line(ui, s);
                        if !f.summary.is_empty() {
                            ui.add(Label::new(RichText::new(&f.summary).size(13.5)).wrap().selectable(true));
                        }
                        for (i, finding) in f.findings.iter().enumerate() {
                            let row = locate(&c.rows, &finding.file);
                            if card_of(ui, i, finding, row.is_some(), widgets) {
                                jump = row.map(|r| (r, finding.line));
                            }
                        }
                        if !f.not_checked.is_empty() {
                            Frame::new()
                                .fill(PANEL())
                                .stroke(Stroke::new(1.0, CARD_LINE()))
                                .corner_radius(8)
                                .inner_margin(Margin::symmetric(13, 10))
                                .show(ui, |ui| {
                                    ui.set_width(ui.available_width());
                                    ui.horizontal_wrapped(|ui| {
                                        ui.spacing_mut().item_spacing.x = 0.0;
                                        ui.label(RichText::new("Not checked: ").font(semibold(12.5)));
                                        ui.label(RichText::new(&f.not_checked).size(12.5).color(MUTED()));
                                    });
                                });
                        }
                    }
                    ReviewReply::Text(text) => {
                        for_line(ui, s);
                        Frame::new()
                            .fill(PANEL())
                            .stroke(Stroke::new(1.0, CARD_LINE()))
                            .corner_radius(8)
                            .inner_margin(Margin::symmetric(14, 12))
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.spacing_mut().item_spacing.y = 6.0;
                                for (p, paragraph) in text.split('\n').enumerate() {
                                    let pieces = pieces(&c.rows, paragraph);
                                    if pieces.iter().all(|(_, place)| place.is_none()) {
                                        ui.add(Label::new(RichText::new(paragraph).size(13.0)).wrap().selectable(true));
                                        continue;
                                    }
                                    ui.horizontal_wrapped(|ui| {
                                        ui.spacing_mut().item_spacing.x = 0.0;
                                        for (k, (text, place)) in pieces.into_iter().enumerate() {
                                            match place {
                                                None => {
                                                    ui.label(RichText::new(text).size(13.0));
                                                }
                                                Some((row, line)) => {
                                                    let r = ui.add(Button::new(RichText::new(text).monospace().size(12.5).color(BLUE())).frame(false));
                                                    pages::control(widgets, format!("review-place-{p}-{k}"), &r, ui.clip_rect());
                                                    if r.clicked() {
                                                        jump = Some((row, Some(line)));
                                                    }
                                                }
                                            }
                                        }
                                    });
                                }
                            });
                        ui.label(RichText::new("Shown as written: the reply didn't come as a list.").size(12.0).color(HINT()));
                    }
                }
            });
        });
    pages::region(widgets, "review-findings-scroll", out.inner_rect.intersect(ui.clip_rect()), true);
    if let Some((row, line)) = jump {
        s.show_findings = false;
        s.show_exact = false;
        s.show_files = false;
        s.selected = Some(row);
        s.mark = line.map(|l| (row, l));
        s.jump = true;
    }
}

/// SUMMARY, and which commits the findings are for.
fn for_line(ui: &mut Ui, s: &State) {
    let Some((base, target)) = &s.result_ids else {
        return;
    };
    let (text, color) = if s.reviewed_now() {
        (format!("For {} → {} · check each one, nothing was changed", &base.as_str()[..7], &target.as_str()[..7]), HINT())
    } else {
        (format!("For {} → {}, not the changes now shown", &base.as_str()[..7], &target.as_str()[..7]), AMBER())
    };
    ui.horizontal(|ui| {
        ui.label(RichText::new("SUMMARY").font(semibold(11.0)).extra_letter_spacing(0.6).color(FAINT()));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.add(Label::new(RichText::new(text).size(12.0).color(color)).truncate());
        });
    });
}

/// One finding: Bug or Check, where, what and why, and a fix when there is one. Returns
/// whether its place was clicked.
fn card_of(ui: &mut Ui, i: usize, f: &ReviewFinding, known: bool, widgets: &mut HashMap<String, UiControl>) -> bool {
    let mut jump = false;
    Frame::new()
        .fill(PANEL())
        .stroke(Stroke::new(1.0, CARD_LINE()))
        .corner_radius(8)
        .inner_margin(Margin::symmetric(14, 12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 6.0;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                let (word, color) = if f.kind == FindingKind::Bug { ("Bug", RED()) } else { ("Check", AMBER()) };
                pages::pill(ui, word, color);
                let at = match f.line {
                    Some(line) if !f.file.is_empty() => format!("{}:{line}", f.file),
                    _ => f.file.clone(),
                };
                if known {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if link(ui, widgets, &format!("review-finding-{i}"), "Show in Changes ›", MUTED()) {
                            jump = true;
                        }
                        ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                            let r = ui.add(Button::new(RichText::new(&at).monospace().size(12.5).color(BLUE())).frame(false));
                            pages::control(widgets, format!("review-finding-{i}-place"), &r, ui.clip_rect());
                            jump |= r.clicked();
                        });
                    });
                } else if !at.is_empty() {
                    ui.add(Label::new(RichText::new(&at).monospace().size(12.5).color(MUTED())).truncate());
                }
            });
            ui.add(Label::new(RichText::new(&f.title).font(semibold(13.5))).wrap().selectable(true));
            if !f.why.is_empty() {
                ui.add(Label::new(RichText::new(&f.why).size(12.5).color(MUTED())).wrap().selectable(true));
            }
            if !f.fix.is_empty() {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    ui.label(RichText::new("Fix: ").font(semibold(12.5)));
                    ui.label(RichText::new(&f.fix).size(12.5));
                });
            }
        });
    jump
}

/// A send's problem with its code under Details, then a stop that needs saying.
fn notes(ui: &mut Ui, s: &mut State, widgets: &mut HashMap<String, UiControl>) {
    if let Some(problem) = &s.problem {
        let mut open = s.details;
        Frame::new()
            .fill(RED().gamma_multiply(0.08))
            .stroke(Stroke::new(1.0, RED().gamma_multiply(0.33)))
            .corner_radius(8)
            .inner_margin(Margin::symmetric(12, 9))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 2.0;
                ui.add(Label::new(RichText::new(&problem.text).size(12.5).color(RED())).wrap());
                if problem.details && link(ui, widgets, "review-details", "Details", RED()) {
                    open = !open;
                }
                if open && problem.details {
                    let text = problem.said.as_ref().map_or_else(|| problem.code.clone(), |said| format!("{}\n{said}", problem.code));
                    ui.add(Label::new(RichText::new(text).monospace().size(11.5).color(MUTED())).wrap().selectable(true));
                }
            });
        s.details = open;
        ui.add_space(10.0);
    }
    if let Some(notice) = &s.notice {
        Frame::new()
            .fill(PANEL())
            .stroke(Stroke::new(1.0, CARD_LINE()))
            .corner_radius(8)
            .inner_margin(Margin::symmetric(12, 9))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.add(Label::new(RichText::new(notice).size(12.5).color(MUTED())).wrap());
            });
        ui.add_space(10.0);
    }
}

fn choose_reviewer(s: &mut State, provider: DraftProvider) {
    s.provider = provider;
    s.chosen = true;
    s.problem = None;
    s.details = false;
}

/// Who reviews, stretched across the card.
fn who_reviews(ui: &mut Ui, s: &mut State, widgets: &mut HashMap<String, UiControl>) {
    let items = [
        ("Claude Code", DraftProvider::Claude, "review-provider-claude"),
        ("Codex", DraftProvider::Codex, "review-provider-codex"),
    ];
    let enabled = !s.sending();
    Frame::new()
        .fill(RAISED())
        .stroke(Stroke::new(1.0, HAIRLINE()))
        .corner_radius(7)
        .inner_margin(2)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            let room = ui.available_width() - 2.0;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                for (label, provider, id) in items {
                    let selected = s.provider == provider;
                    let text = RichText::new(label).size(12.5).color(if selected { TEXT() } else { MUTED() });
                    let button = Button::new(text)
                        .selected(selected)
                        .fill(if selected { SELECTED() } else { Color32::TRANSPARENT })
                        .corner_radius(5)
                        .min_size(vec2(room / 2.0, 24.0));
                    let r = ui.add_enabled(enabled, quiet_unless(selected, button));
                    pages::control(widgets, id, &r, ui.clip_rect());
                    if r.clicked() && !selected {
                        choose_reviewer(s, provider);
                    }
                }
            });
        });
}

/// Who gets the changes, and what stands in the way. A narrow card leaves out the account line.
fn goes_to(ui: &mut Ui, s: &State, check: &ProgramCheck, last: bool, compact: bool, intents: &mut Vec<Intent>, widgets: &mut HashMap<String, UiControl>) {
    let (to, pays, name, program) = match s.provider {
        DraftProvider::Claude => ("Anthropic, through Claude Code on this computer", "Your Claude plan or API usage applies", "Claude Code", "claude.exe"),
        DraftProvider::Codex => ("OpenAI, through Codex on this computer", "Your ChatGPT or OpenAI account applies", "Codex", "codex.exe"),
    };
    let side = |ui: &mut Ui| match check {
        ProgramCheck::Found(path) => {
            found(ui, "Found");
            ui.interact(ui.min_rect(), Id::new("review-found"), Sense::hover()).on_hover_text(path.as_str());
        }
        _ => {
            pages::pill(ui, "Not found", AMBER());
        }
    };
    field(ui, "Goes to", last, side, |ui| {
        ui.add(Label::new(to).wrap());
        let offer = match check {
            ProgramCheck::Found(_) => {
                if !compact {
                    ui.label(RichText::new(pays).size(12.0).color(HINT()));
                }
                None
            }
            ProgramCheck::Missing => Some(format!("Install {name}, or choose its {program}.")),
            ProgramCheck::Problem(text) => Some(text.clone()),
        };
        if let Some(text) = offer
            && !s.sending()
            && fix(ui, widgets, &text, "review-browse", "Browse")
        {
            intent(intents, ReviewIntent::Browse);
        }
    });
}

/// What a shortened send leaves out.
fn shortened(rows: &[Row]) -> String {
    let named = rows.iter().filter(|r| r.sent == Sent::Named).count();
    let cut = rows.iter().any(|r| matches!(r.sent, Sent::Cut(_)));
    let files = |n: usize| if n == 1 { "1 file is".to_owned() } else { format!("{n} files are") };
    match (named, cut) {
        (0, _) => "Shortened to fit: only the start of the diff is sent.".into(),
        (n, false) => format!("Shortened to fit. {} named only.", files(n)),
        (n, true) => format!("Shortened to fit: one file is cut short and {} named only.", files(n)),
    }
}

/// What a send carries, and the exact data on request; a narrow card says it shorter.
fn sends(ui: &mut Ui, s: &mut State, last: bool, compact: bool, widgets: &mut HashMap<String, UiControl>) {
    let changes = s.changes.clone().filter(|_| !s.changed_inputs());
    let mut show = s.show_exact;
    let size = |ui: &mut Ui| {
        if let Some(c) = &changes {
            let scope = &c.preview.scope;
            let text = if scope.shortened() {
                format!("{} of {}", pages_size(scope.sent_bytes), pages_size(scope.diff_bytes))
            } else {
                pages_size(c.preview.payload().len())
            };
            ui.label(RichText::new(text).size(12.0).color(HINT()));
        }
    };
    let provider = s.provider;
    let (base, target) = (s.base.trim().to_owned(), s.target.trim().to_owned());
    field(ui, "Sends", last, size, |ui| {
        let text = match (base.is_empty() || target.is_empty(), compact) {
            (true, _) => "The commits on a branch since it split from its base: file names and diff.".to_owned(),
            (false, true) => format!("The commits on {target} since {base}"),
            (false, false) => format!("The commits on {target} since {base}: file names and diff. Binary files are named only."),
        };
        ui.add(Label::new(text).wrap());
        if let Some(c) = changes.as_ref().filter(|c| c.preview.scope.shortened()) {
            Frame::new()
                .fill(AMBER().gamma_multiply(0.10))
                .stroke(Stroke::new(1.0, AMBER().gamma_multiply(0.30)))
                .corner_radius(8)
                .inner_margin(Margin::symmetric(10, 6))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.add(Label::new(RichText::new(shortened(&c.rows)).size(12.5).color(AMBER())).wrap());
                });
        }
        if provider == DraftProvider::Codex {
            ui.label(RichText::new("Codex may add your global AGENTS instructions. Its tools are off for this request.").size(12.0).color(HINT()));
        }
        let label = if show { "Show the changes" } else { "Show exact data" };
        if changes.is_some() && link(ui, widgets, "review-exact", label, BLUE()) {
            show = !show;
        }
    });
    if show != s.show_exact {
        s.show_exact = show;
        if show {
            s.show_findings = false;
        }
    }
}

/// The running review's row at the foot of the card: the time, and when it gives up.
fn progress(ui: &mut Ui, s: &State) {
    card_foot(ui, Some(CARD()), |ui| {
        ui.label(RichText::new(format!("Stops after {} min", DRAFT_TIMEOUT.as_secs() / 60)).size(11.5).color(HINT()));
        ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            ui.add(Spinner::new().size(14.0));
            let text = format!("{} is reviewing · {}", s.asked.label(), elapsed(s.started));
            ui.add(Label::new(RichText::new(text).size(12.5)).truncate());
        });
    });
}

/// The Send for review card in the column of a wide page.
fn card(ui: &mut Ui, s: &mut State, check: &ProgramCheck, intents: &mut Vec<Intent>, widgets: &mut HashMap<String, UiControl>) {
    let sending = s.sending();
    card_frame(ui, |ui| {
        card_header(ui, "Send for review", |_| {});
        field(ui, "Who reviews", false, |_| {}, |ui| who_reviews(ui, s, widgets));
        goes_to(ui, s, check, sending, false, intents, widgets);
        if sending {
            progress(ui, s);
        } else {
            sends(ui, s, true, false, widgets);
        }
    });
}

/// A narrow page's card: who reviews in its title bar, then where it goes and what it
/// sends side by side.
fn stacked_card(ui: &mut Ui, s: &mut State, check: &ProgramCheck, intents: &mut Vec<Intent>, widgets: &mut HashMap<String, UiControl>) {
    let sending = s.sending();
    card_frame(ui, |ui| {
        card_header(ui, "Send for review", |ui| {
            let items = [("Claude Code", s.provider == DraftProvider::Claude), ("Codex", s.provider == DraftProvider::Codex)];
            let ids = ["review-provider-claude".to_owned(), "review-provider-codex".to_owned()];
            let picked = ui.add_enabled_ui(!sending, |ui| choices(ui, widgets, &items, &ids)).inner;
            if let Some(i) = picked {
                choose_reviewer(s, [DraftProvider::Claude, DraftProvider::Codex][i]);
            }
        });
        if sending {
            goes_to(ui, s, check, true, true, intents, widgets);
            progress(ui, s);
        } else {
            // Columns justify wrapped text; each field lays out ragged-right instead.
            ui.columns(2, |cols| {
                cols[0].with_layout(Layout::top_down(Align::Min), |ui| goes_to(ui, s, check, true, true, intents, widgets));
                cols[1].with_layout(Layout::top_down(Align::Min), |ui| sends(ui, s, true, true, widgets));
            });
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: &str, change: Change, original: Option<&str>) -> ComparedFile {
        ComparedFile {
            path: RepoPath::new(path.as_bytes().to_vec()).unwrap(),
            original_path: original.map(|p| RepoPath::new(p.as_bytes().to_vec()).unwrap()),
            change,
            similarity: original.map(|_| 100),
        }
    }

    const DIFF: &str = "diff --git a/src/a.rs b/src/a.rs\nindex 1..2 100644\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -1,2 +1,2 @@\n fn a() {\n-old\n+new\ndiff --git a/logo.png b/logo.png\nnew file mode 100644\nindex 0..1\nBinary files /dev/null and b/logo.png differ\ndiff --git a/src/old.rs b/src/new.rs\nsimilarity index 100%\nrename from src/old.rs\nrename to src/new.rs\n";

    fn sample() -> Vec<Row> {
        let files = [
            file("src/a.rs", Change::Modified, None),
            file("logo.png", Change::Added, None),
            file("src/new.rs", Change::Renamed, Some("src/old.rs")),
            file("big/data.csv", Change::Added, None),
        ];
        rows(&files, &Patch::parse(DIFF.as_bytes()), true)
    }

    #[test]
    fn each_file_is_bound_to_what_of_it_is_sent() {
        let rows = sample();
        assert_eq!(rows.iter().map(|r| r.sent).collect::<Vec<_>>(), [Sent::Diff(0), Sent::Binary, Sent::Diff(2), Sent::Named]);
        assert_eq!(rows[2].label, "src/new.rs");
    }

    #[test]
    fn a_diff_that_stops_inside_its_last_hunk_is_cut() {
        let whole = Patch::parse(b"diff --git a/x b/x\n--- a/x\n+++ b/x\n@@ -1,3 +1,3 @@\n a\n-b\n+c\n d\n");
        assert!(!cut(&whole.files[0]));
        let cut_short = Patch::parse(b"diff --git a/x b/x\n--- a/x\n+++ b/x\n@@ -1,3 +1,3 @@\n a\n-b\n");
        assert!(cut(&cut_short.files[0]));
        let single = Patch::parse(b"diff --git a/x b/x\n--- a/x\n+++ b/x\n@@ -1 +1 @@\n-a\n+b\n");
        assert!(!cut(&single.files[0]), "a count of one may be left out");
        let files = [file("x", Change::Modified, None)];
        assert_eq!(rows(&files, &cut_short, true)[0].sent, Sent::Cut(0));
        assert_eq!(rows(&files, &cut_short, false)[0].sent, Sent::Diff(0), "only a shortened send cuts a file");
    }

    #[test]
    fn a_named_file_is_found_by_its_path_or_the_one_path_ending_in_it() {
        let rows = sample();
        assert_eq!(locate(&rows, "src/a.rs"), Some(0));
        assert_eq!(locate(&rows, "`./src/a.rs`"), Some(0));
        assert_eq!(locate(&rows, "b/src/new.rs"), Some(2));
        assert_eq!(locate(&rows, "a.rs"), Some(0));
        assert_eq!(locate(&rows, "data.csv"), Some(3));
        assert_eq!(locate(&rows, "src\\new.rs"), Some(2));
        assert_eq!(locate(&rows, "missing.rs"), None);
        assert_eq!(locate(&rows, ""), None);
        assert_eq!(locate(&rows, "rs"), None, "a part of a name is not a path");
    }

    #[test]
    fn places_in_plain_text_become_links_and_the_rest_stays_text() {
        let rows = sample();
        let text = "One thing: src/a.rs:2 has no limit (see a.rs:1, and nope.rs:3).";
        let pieces = pieces(&rows, text);
        let links: Vec<_> = pieces.iter().filter_map(|(t, p)| p.map(|p| (*t, p))).collect();
        assert_eq!(links, [("src/a.rs:2", (0, 2)), ("a.rs:1", (0, 1))]);
        assert_eq!(pieces.iter().map(|(t, _)| *t).collect::<String>(), text, "nothing is lost");
        assert_eq!(place(&rows, "src/a.rs:2:7"), Some((0, 2)), "a column is allowed");
        assert_eq!(place(&rows, "src/a.rs:0"), None);
        assert_eq!(place(&rows, "src/a.rs"), None, "a link needs a line");
    }

    #[test]
    fn copied_findings_read_as_text() {
        let f = ReviewFindings {
            summary: "One bug.".into(),
            findings: vec![ReviewFinding {
                kind: FindingKind::Bug,
                file: "src/a.rs".into(),
                line: Some(2),
                title: "Underflow".into(),
                why: "It wraps.".into(),
                fix: "Check first.".into(),
            }],
            not_checked: "Tests.".into(),
        };
        assert_eq!(findings_text(&f), "One bug.\n\nBug: Underflow (src/a.rs:2)\nIt wraps.\nFix: Check first.\n\nNot checked: Tests.");
    }

    #[test]
    fn the_page_opens_on_the_current_branch_against_main_and_asks_on_main_itself() {
        let branch = |name: &str, head: bool, kind: BranchKind| Branch {
            reference: name.as_bytes().to_vec(),
            target: ObjectId::parse(&"0".repeat(40)).unwrap(),
            kind,
            is_head: head,
            upstream: None,
            symbolic_target: name.ends_with("/HEAD").then(|| b"refs/remotes/origin/main".to_vec()),
            checked_out_at: None,
        };
        let on = |current: &str| {
            [
                branch("refs/heads/main", current == "main", BranchKind::Local),
                branch("refs/heads/retry-login", current == "retry-login", BranchKind::Local),
                branch("refs/remotes/origin/main", false, BranchKind::Remote),
                branch("refs/remotes/origin/HEAD", false, BranchKind::Remote),
            ]
        };
        assert_eq!(defaults(&on("retry-login")), ("main".into(), "retry-login".into()));
        assert_eq!(defaults(&on("main")), ("main".into(), String::new()), "never main against itself");
        assert_eq!(defaults(&on("")), ("main".into(), String::new()), "a detached HEAD asks too");
        assert_eq!(branch_names(&on("main")), ["main", "retry-login", "origin/main"]);
        let master = [branch("refs/heads/master", false, BranchKind::Local), branch("refs/heads/fix", true, BranchKind::Local)];
        assert_eq!(defaults(&master), ("master".into(), "fix".into()));
    }

    #[test]
    fn failures_speak_of_reviews_and_keep_their_codes_for_details() {
        for provider in [DraftProvider::Claude, DraftProvider::Codex] {
            let failed = |failure, detail: Option<&str>| {
                problem(&Error::CommitDraft { provider, failure, detail: detail.map(str::to_owned) })
            };
            for failure in [DraftFailure::InvalidReply, DraftFailure::TimedOut, DraftFailure::SignInRequired, DraftFailure::ClaudeFailed] {
                let shown = failed(failure, None);
                assert_eq!(shown.code, failure.code());
                assert!(shown.text.starts_with(provider.label()), "{}", shown.text);
                assert!(!shown.text.contains("draft") && !shown.text.contains('['), "{}", shown.text);
            }
            assert!(failed(DraftFailure::TimedOut, None).text.contains("within 2 minutes"));
            assert_eq!(failed(DraftFailure::ClaudeFailed, Some("out of credit")).said.as_deref(), Some("out of credit"));
            assert!(failed(DraftFailure::InvalidReply, None).details);
            assert!(!failed(DraftFailure::TimedOut, None).details);
        }
        assert_eq!(code(&Error::BranchReview(BranchReviewFailure::TargetNotFound)), "review_target_not_found");
        assert!(read_problem(&Error::BranchReview(BranchReviewFailure::NoUniqueBase), "main", "fix").starts_with("fix and main"));
    }

    #[test]
    fn a_shortened_send_says_what_it_leaves_out() {
        let said = |sent: &[Sent]| {
            shortened(&sent.iter().map(|s| Row { file: file("x", Change::Modified, None), label: "x".into(), sent: *s }).collect::<Vec<_>>())
        };
        assert_eq!(said(&[Sent::Diff(0), Sent::Named]), "Shortened to fit. 1 file is named only.");
        assert_eq!(said(&[Sent::Named, Sent::Named, Sent::Diff(0)]), "Shortened to fit. 2 files are named only.");
        assert_eq!(said(&[Sent::Cut(0)]), "Shortened to fit: only the start of the diff is sent.");
        assert_eq!(said(&[Sent::Cut(0), Sent::Named]), "Shortened to fit: one file is cut short and 1 file is named only.");
    }

    #[test]
    fn send_waits_for_the_program_the_changes_and_settled_fields() {
        let mut s = State::default();
        assert!(!s.can_send(true), "nothing read yet");
        s.base = "main".into();
        s.target = "fix".into();
        s.edited = Some(Instant::now());
        assert!(!s.can_send(true));
        assert!(s.changed_inputs());
    }

    #[test]
    fn the_first_program_found_reviews() {
        assert_eq!(first_found(|_| true), DraftProvider::Claude);
        assert_eq!(first_found(|p| p == DraftProvider::Codex), DraftProvider::Codex);
        assert_eq!(first_found(|_| false), DraftProvider::Claude, "Not found offers Browse");
    }
}
