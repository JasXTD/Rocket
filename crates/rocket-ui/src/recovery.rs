//! Reflog & recovery: a timeline of where HEAD and branches have been, in plain
//! words, beside the selected entry's details. Making a branch there or moving the
//! current branch back happens in the details pane, after a review. All repository
//! work stays on workers.
use crate::{
    app::*,
    backend::{Action, Outcome, user_error},
    pages::{self, control},
    style::*,
    tags::check,
};
use eframe::egui::*;
use rocket_git::*;
use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc,
        mpsc::{Receiver, sync_channel},
    },
    time::{Duration, Instant},
};

/// How long the branch name's review waits after typing.
const SETTLE: Duration = Duration::from_millis(400);
/// The details pane's width.
const PANE: f32 = 380.0;

/// Backend progress remains typed; only its UI presentation is translated.
fn progress_label(stage: &LostCommitProgress) -> String {
    match stage {
        LostCommitProgress::Inspecting => crate::i18n::tr("Checking repository history…").into(),
        LostCommitProgress::Scanning => crate::i18n::tr("Scanning objects not reached by refs or reflogs…").into(),
        LostCommitProgress::ReadingCommits(count) => crate::i18n::trf!("Reading commit records: {count}…", count),
        LostCommitProgress::Complete(count) => crate::i18n::trf!("Scan complete · lost commits: {count}", count),
    }
}

pub struct RecoveryRead {
    page: ReflogPage,
    lost: Option<Arc<LostCommitScan>>,
    off: Option<HashSet<ObjectId>>,
}

pub struct DialogState {
    pub scope: ReflogScope,
    pub page: Option<ReflogPage>,
    pub log_task: Option<ReadTask<RecoveryRead>>,
    pub lost_scan: Option<Arc<LostCommitScan>>,
    pub scan_stage: Option<String>,
    pub read_notice: Option<String>,
    progress: Option<Receiver<LostCommitProgress>>,
    started: Option<Instant>,
    pub task: Option<ReadTask<RecoveryPreview>>,
    pub preview: Option<Arc<RecoveryPreview>>,
    pub target: Option<ObjectId>,
    pub selected_row: Option<usize>,
    pub action: Option<RecoveryAction>,
    pub limit: usize,
    pub applying: bool,
    /// The listed commits no branch reaches, when known.
    pub off: Option<HashSet<ObjectId>>,
    /// Narrows the list by words or commit ID.
    pub filter: String,
    details_task: Option<(ObjectId, ReadTask<RecoveryDetails>)>,
    /// The selected commit, read for the details pane.
    pub details: Option<(ObjectId, std::result::Result<Arc<RecoveryDetails>, String>)>,
    show_diff: bool,
    /// When the recovery branch name was last edited; its review waits for a pause.
    edited: Option<Instant>,
}
impl DialogState {
    /// A read or review is running.
    pub(crate) fn reading(&self) -> bool {
        self.task.is_some() || self.log_task.is_some() || self.details_task.is_some()
    }
}
impl Default for DialogState {
    fn default() -> Self {
        Self {
            scope: ReflogScope::Head,
            page: None,
            log_task: None,
            lost_scan: None,
            scan_stage: None,
            read_notice: None,
            progress: None,
            started: None,
            task: None,
            preview: None,
            target: None,
            selected_row: None,
            action: None,
            limit: 200,
            applying: false,
            off: None,
            filter: String::new(),
            details_task: None,
            details: None,
            show_diff: false,
            edited: None,
        }
    }
}

/// What a scope lists, for the scopes whose name does not say it.
fn scope_note(scope: ReflogScope) -> Option<&'static str> {
    match scope {
        ReflogScope::Head | ReflogScope::AllRefs => None,
        ReflogScope::DeletedBranches => Some(
            crate::i18n::tr("Branches this worktree switched away from that no longer exist, at the tip each had when you left it."),
        ),
        ReflogScope::Retained => Some(
            crate::i18n::tr("Commits Rocket kept before resets, rebases, amends, stopped sequences, tag deletions and worktree removals."),
        ),
        ReflogScope::Unreachable => Some(
            crate::i18n::tr("Commits no branch, tag or reflog reaches, such as dropped stashes. Only a new branch can bring one back."),
        ),
    }
}

fn scope_empty(scope: ReflogScope) -> &'static str {
    match scope {
        ReflogScope::Head | ReflogScope::AllRefs => crate::i18n::tr("No entries in this history yet."),
        ReflogScope::DeletedBranches => crate::i18n::tr("No deleted branches found in this worktree's history."),
        ReflogScope::Retained => crate::i18n::tr("Rocket has not kept any commits in this repository."),
        ReflogScope::Unreachable => crate::i18n::tr("No lost commits found."),
    }
}

const MONTHS: [&str; 12] = [
    crate::i18n::mark("Jan"), crate::i18n::mark("Feb"), crate::i18n::mark("Mar"), crate::i18n::mark("Apr"), crate::i18n::mark("May"), crate::i18n::mark("Jun"), crate::i18n::mark("Jul"), crate::i18n::mark("Aug"), crate::i18n::mark("Sep"), crate::i18n::mark("Oct"), crate::i18n::mark("Nov"), crate::i18n::mark("Dec"),
];

/// The date, time and UTC offset in minutes of a dated reflog selector such as
/// `HEAD@{2026-10-25T12:40:30+08:00}`.
fn selector_time(selector: &[u8]) -> Option<(String, String, i32)> {
    let text = std::str::from_utf8(selector).ok()?;
    let inner = text.split_once("@{")?.1.strip_suffix('}')?;
    if inner.len() < 25 || inner.as_bytes().get(10) != Some(&b'T') {
        return None;
    }
    let sign = match inner.as_bytes()[19] {
        b'+' => 1,
        b'-' => -1,
        _ => return None,
    };
    let hours: i32 = inner.get(20..22)?.parse().ok()?;
    let minutes: i32 = inner.get(23..25)?.parse().ok()?;
    Some((inner[..10].to_owned(), inner[11..16].to_owned(), sign * (hours * 60 + minutes)))
}

/// "Today", "Yesterday", "Oct 23" or "Oct 23, 2025" for a `YYYY-MM-DD` date in a
/// time zone `offset` minutes from UTC.
fn day_label(day: &str, offset: i32) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    if date(now, offset).starts_with(day) {
        return crate::i18n::tr("Today").into();
    }
    if date(now - 86_400, offset).starts_with(day) {
        return crate::i18n::tr("Yesterday").into();
    }
    let month = day.get(5..7).and_then(|m| m.parse::<usize>().ok()).filter(|m| (1..=12).contains(m));
    let number = day.get(8..10).and_then(|d| d.parse::<u32>().ok());
    match (month, number) {
        (Some(m), Some(n)) if date(now, offset).starts_with(&day[..4]) => crate::i18n::trf!("{month} {day}", month = crate::i18n::tr(MONTHS[m - 1]), day = n),
        (Some(m), Some(n)) => crate::i18n::trf!("{month} {day}, {year}", month = crate::i18n::tr(MONTHS[m - 1]), day = n, year = &day[..4]),
        _ => day.to_owned(),
    }
}

/// The group an entry is listed under, and its time on the right.
fn when(entry: &ReflogEntry, scope: ReflogScope) -> (String, String) {
    match (scope, selector_time(&entry.selector)) {
        (ReflogScope::Retained, _) => (crate::i18n::tr("Kept by Rocket").into(), String::new()),
        (ReflogScope::Unreachable, _) => (crate::i18n::tr("Found by the scan").into(), String::new()),
        (_, Some((day, time, offset))) => (day_label(&day, offset), time),
        (_, None) => (crate::i18n::tr("Earlier").into(), String::new()),
    }
}

/// What an entry records, in words, and its icon.
fn describe(entry: &ReflogEntry, scope: ReflogScope) -> (String, Icon) {
    let message = String::from_utf8_lossy(&entry.subject);
    match scope {
        ReflogScope::DeletedBranches => (
            entry.reference.as_deref().map(display).unwrap_or_else(|| message.to_string()),
            Icon::Branch,
        ),
        ReflogScope::Retained => {
            let reference = entry.reference.as_deref().map(display).unwrap_or_default();
            let kind = reference.strip_prefix("refs/rocket/").unwrap_or(&reference);
            let what = match kind.split('/').take(2).collect::<Vec<_>>().as_slice() {
                ["recovery", "reset"] => crate::i18n::tr("Before a reset"),
                ["recovery", "rebase"] => crate::i18n::tr("Before a rebase"),
                ["recovery", "amend"] => crate::i18n::tr("Before an amend"),
                ["recovery", "sequence"] => crate::i18n::tr("Before a stopped cherry-pick or revert"),
                ["recovery", "mail-import"] => crate::i18n::tr("Before a commit series import"),
                ["deleted-tags", ..] => crate::i18n::tr("A deleted tag"),
                ["removed-worktrees", ..] => crate::i18n::tr("A removed worktree's branch"),
                ["stash", ..] | ["stash-snapshot", ..] => crate::i18n::tr("A stash snapshot"),
                _ => crate::i18n::tr("Kept by Rocket"),
            };
            (what.into(), Icon::Clock)
        }
        ReflogScope::Unreachable => (message.to_string(), Icon::Commit),
        ReflogScope::Head | ReflogScope::AllRefs => {
            let (verb, rest) = message.split_once(": ").unwrap_or((&message, ""));
            let words = |text: &str| text.trim().to_owned();
            match verb {
                "reset" => (
                    crate::i18n::trf!("Reset to {target}", target = words(rest.strip_prefix("moving to ").unwrap_or(rest))),
                    Icon::Refresh,
                ),
                "checkout" => match rest.strip_prefix("moving from ").and_then(|r| r.split_once(" to ")) {
                    Some((_, to)) => (crate::i18n::trf!("Switched to {target}", target = words(to)), Icon::Branch),
                    None => (crate::i18n::tr("Switched").into(), Icon::Branch),
                },
                "commit" => (crate::i18n::tr("Committed").into(), Icon::Commit),
                "commit (amend)" => (crate::i18n::tr("Amended").into(), Icon::Commit),
                "commit (initial)" => (crate::i18n::tr("First commit").into(), Icon::Commit),
                "commit (merge)" => (crate::i18n::tr("Merge commit").into(), Icon::Merge),
                "pull" => (crate::i18n::tr("Pulled").into(), Icon::Pull),
                "clone" => (crate::i18n::tr("Cloned").into(), Icon::Fetch),
                "cherry-pick" => (crate::i18n::tr("Cherry-picked").into(), Icon::Commit),
                "revert" => (crate::i18n::tr("Reverted").into(), Icon::Commit),
                "branch" => (crate::i18n::tr("Created a branch").into(), Icon::Branch),
                _ if verb.starts_with("merge ") => (crate::i18n::trf!("Merged {branch}", branch = words(&verb[6..])), Icon::Merge),
                _ if verb.starts_with("pull") => (crate::i18n::tr("Pulled").into(), Icon::Pull),
                _ if verb.starts_with("rebase") => (
                    if verb.contains("finish") { crate::i18n::tr("Rebased").into() } else { crate::i18n::tr("Rebasing").into() },
                    Icon::Branch,
                ),
                _ => {
                    let mut chars = message.chars();
                    let text = match chars.next() {
                        Some(first) => first.to_uppercase().chain(chars).collect(),
                        None => String::new(),
                    };
                    (text, Icon::Commit)
                }
            }
        }
    }
}

/// The subject of the commit an entry points to, or for lost commits their author.
fn entry_subject(entry: &ReflogEntry, scope: ReflogScope) -> String {
    if scope == ReflogScope::Unreachable {
        return display(&entry.actor);
    }
    let subject = if entry.commit_subject.is_empty() { &entry.subject } else { &entry.commit_subject };
    String::from_utf8_lossy(crate::text::prefix(subject, 1024)).chars().take(240).collect()
}

impl Rocket {
    pub(crate) fn start_recovery_read(&mut self) {
        let Some(d) = &mut self.dialog else { return };
        if !matches!(d.kind, DialogKind::Recovery) {
            return;
        }
        let s = d.recovery.get_or_insert_with(Default::default);
        if s.applying {
            return;
        }
        d.error = None;
        s.task = None;
        s.log_task = None;
        s.preview = None;
        s.progress = None;
        s.read_notice = None;
        s.started = Some(Instant::now());
        let Some(backend) = self.backend.clone() else {
            return;
        };
        let Some(path) = self
            .tabs
            .iter()
            .find(|t| Some(t.id) == d.owner)
            .map(|t| t.path.clone())
        else {
            return;
        };
        if let Some(mut action) = s.action.clone() {
            let Some(target) = s.target.clone() else {
                return;
            };
            if let RecoveryAction::Branch(name) = &mut action {
                *name = d.first.trim().to_owned();
            }
            s.action = Some(action.clone());
            let lost = s
                .lost_scan
                .clone()
                .filter(|_| s.scope == ReflogScope::Unreachable);
            match ReadTask::spawn(move |cancel| {
                if let Some(scan) = lost {
                    let RecoveryAction::Branch(name) = action else {
                        return Err(Error::InvalidInput(
                            crate::i18n::tr("Lost commits require a recovery branch").into(),
                        ));
                    };
                    return backend.lost_recovery_preview(&path, &scan, &target, &name, &cancel);
                }
                backend.recovery_preview(&path, &target, action, &cancel)
            }) {
                Ok(task) => s.task = Some(task),
                Err(e) => d.error = Some(user_error(&e)),
            }
        } else {
            // The selected commit is read again too: its place against the branch may
            // have changed.
            s.details = None;
            s.details_task = None;
            let limit = s.limit;
            let scope = s.scope;
            let (sender, receiver) = sync_channel(8);
            s.progress = Some(receiver);
            s.scan_stage = None;
            match ReadTask::spawn(move |cancel| {
                if scope == ReflogScope::Unreachable {
                    let scan = Arc::new(backend.lost_commits(&path, &cancel, &sender)?);
                    Ok(RecoveryRead {
                        page: scan.page(limit)?,
                        lost: Some(scan),
                        off: None,
                    })
                } else {
                    let page = backend.reflog_scoped(&path, scope, limit, &cancel)?;
                    let ids: Vec<ObjectId> = page.entries.iter().map(|e| e.id.clone()).collect();
                    let off = backend.off_branches(&path, &ids, &cancel)?;
                    Ok(RecoveryRead {
                        page,
                        lost: None,
                        off,
                    })
                }
            }) {
                Ok(task) => s.log_task = Some(task),
                Err(e) => d.error = Some(user_error(&e)),
            }
        }
    }

    /// Read the selected entry's commit for the details pane, once per selection.
    fn start_recovery_details(&mut self) {
        let Some(d) = &mut self.dialog else { return };
        let Some(s) = &mut d.recovery else { return };
        let Some(target) = s.target.clone() else {
            return;
        };
        if s.scope == ReflogScope::Unreachable
            || s.applying
            || s.details.as_ref().is_some_and(|(id, _)| *id == target)
            || s.details_task.as_ref().is_some_and(|(id, _)| *id == target)
        {
            return;
        }
        let (Some(backend), Some(path)) = (
            self.backend.clone(),
            self.tabs.iter().find(|t| Some(t.id) == d.owner).map(|t| t.path.clone()),
        ) else {
            return;
        };
        let id = target.clone();
        match ReadTask::spawn(move |cancel| backend.recovery_details(&path, &id, &cancel)) {
            Ok(task) => s.details_task = Some((target, task)),
            Err(e) => s.details = Some((target, Err(user_error(&e)))),
        }
    }

    pub(crate) fn poll_recovery_read(&mut self) {
        let Some(d) = &mut self.dialog else { return };
        let Some(s) = &mut d.recovery else { return };
        if let Some(progress) = &s.progress {
            for stage in progress.try_iter() {
                s.scan_stage = Some(progress_label(&stage));
            }
        }
        if let Some(result) = s.log_task.as_mut().and_then(ReadTask::try_finish) {
            s.log_task = None;
            s.progress = None;
            match result {
                Ok(RecoveryRead { page, lost, off }) => {
                    if let Some(scan) = &lost {
                        s.scan_stage =
                            Some(progress_label(&LostCommitProgress::Complete(scan.commits().len())));
                    }
                    s.lost_scan = lost;
                    s.off = off;
                    // Several events can point at one commit. Preserve one event
                    // highlight across reloads, while writes remain pinned by OID.
                    let previous = s.selected_row.and_then(|i| s.page.as_ref()?.entries.get(i));
                    s.selected_row = s.target.as_ref().and_then(|id| {
                        page.entries
                            .iter()
                            .position(|e| {
                                e.id == *id
                                    && previous.is_some_and(|old| {
                                        e.selector == old.selector && e.subject == old.subject
                                    })
                            })
                            .or_else(|| page.entries.iter().position(|e| e.id == *id))
                    });
                    if s.selected_row.is_none() {
                        s.target = None;
                    }
                    s.page = Some(page);
                }
                Err(e) => {
                    d.error = Some(user_error(&e));
                    if s.scope == ReflogScope::Unreachable {
                        s.scan_stage = None;
                        s.read_notice = Some(
                            if s.lost_scan.is_some() {
                                crate::i18n::tr("The new scan did not complete. Showing previous completed results.")
                            } else {
                                crate::i18n::tr("No completed scan is available. This is not an empty result.")
                            }
                            .into(),
                        );
                    } else {
                        s.page = None;
                        s.target = None;
                        s.selected_row = None;
                    }
                }
            }
        }
        if let Some(result) = s.task.as_mut().and_then(ReadTask::try_finish) {
            s.task = None;
            match result {
                Ok(p) => s.preview = Some(Arc::new(p)),
                Err(e) => d.error = Some(user_error(&e)),
            }
        }
        let finished = match &mut s.details_task {
            Some((id, task)) => task.try_finish().map(|r| (id.clone(), r)),
            None => None,
        };
        if let Some((id, result)) = finished {
            s.details_task = None;
            s.details = Some((id, result.map(Arc::new).map_err(|e| user_error(&e))));
        }
    }
    pub(crate) fn finish_recovery_dialog(&mut self, owner: Option<u64>, outcome: &Outcome) {
        let Some(d) = &mut self.dialog else { return };
        let Some(s) = &mut d.recovery else { return };
        if !s.applying || d.owner != owner {
            return;
        }
        if outcome.command_succeeded && !outcome.recovery_pending {
            self.dialog = None;
        } else {
            s.applying = false;
            s.preview = None;
            d.error = Some(crate::i18n::trf!(
                "{summary}\n{details}\nRefresh and review before retrying.",
                summary = outcome.summary,
                details = outcome.details.join("\n")
            ));
        }
    }
    pub(crate) fn recovery_dialog(&mut self, ctx: &Context) {
        let Some(mut d) = self.dialog.take() else {
            return;
        };
        let idle = self.write.is_none();
        let running = pages::Running::of(self.write.as_ref());
        let snapshot = self
            .tabs
            .iter()
            .find(|t| Some(t.id) == d.owner)
            .and_then(|t| t.snapshot.as_ref());
        let current = head_name(snapshot.and_then(|s| s.head.as_ref()));
        let (head_target, on_branch) = match snapshot.and_then(|s| s.head.as_ref()) {
            Some(Head::Branch { target, .. }) => (Some(target.clone()), true),
            Some(Head::Detached { target }) => (Some(target.clone()), false),
            _ => (None, false),
        };
        let branches: Vec<Branch> = snapshot.map(|s| s.branches.clone()).unwrap_or_default();
        let s = d.recovery.get_or_insert_with(Default::default);
        let (mut dismiss, mut refresh, mut submit, mut back, mut cancel) =
            (false, false, false, false, false);
        let mut cancel_read = false;
        let waiting = s.task.is_some() || s.log_task.is_some() || s.applying;
        let browsing = s.action.is_none();
        // The branch name's review starts once typing pauses.
        if matches!(s.action, Some(RecoveryAction::Branch(_)))
            && s.task.is_none()
            && s.preview.is_none()
            && !s.applying
            && s.edited.is_some_and(|at| at.elapsed() >= SETTLE)
        {
            s.edited = None;
            let name = d.first.trim();
            refresh = !name.is_empty()
                && crate::small_dialogs::branch_name_problem(name, &branches, None).is_none();
        }
        if s.edited.is_some() || s.log_task.is_some() || s.task.is_some() || s.details_task.is_some() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
        let head = pages::ReviewHeader {
            image: Icon::Clock,
            title: crate::i18n::tr("Reflog & recovery"),
            subtitle: crate::i18n::tr("Find a commit you moved away from, then make a branch there or move your branch back to it."),
            danger: false,
        };
        let height = (ctx.content_rect().height() - 340.0).clamp(220.0, 600.0);
        let response = pages::full_review(
            ctx,
            "rocket-recovery",
            head,
            &mut self.frame_widgets,
            "dialog-recovery-scroll",
            !s.applying,
            |ui, part, widgets| match part {
                pages::Part::Body => {
                    if browsing && let Some(error) = &d.error {
                        pages::callout(ui, RED(), error);
                    }
                    // Where to look, and a filter.
                    ui.horizontal(|ui| {
                        let scopes = [
                            (ReflogScope::Head, crate::i18n::tr("This worktree"), "head"),
                            (ReflogScope::AllRefs, crate::i18n::tr("All branches"), "all"),
                            (ReflogScope::DeletedBranches, crate::i18n::tr("Deleted branches"), "deleted"),
                            (ReflogScope::Retained, crate::i18n::tr("Kept by Rocket"), "retained"),
                            (ReflogScope::Unreachable, crate::i18n::tr("Lost commits"), "unreachable"),
                        ];
                        let items: Vec<(&str, bool)> =
                            scopes.iter().map(|(scope, label, _)| (*label, s.scope == *scope)).collect();
                        ui.add_enabled_ui(!s.applying, |ui| {
                            for (r, (scope, _, id)) in pages::segments(ui, &items).iter().zip(&scopes) {
                                control(widgets, format!("dialog-reflog-scope-{id}"), r, ui.clip_rect());
                                if r.clicked() && s.scope != *scope {
                                    s.scope = *scope;
                                    s.log_task = None;
                                    s.lost_scan = None;
                                    s.progress = None;
                                    s.scan_stage = None;
                                    s.read_notice = None;
                                    s.page = None;
                                    s.off = None;
                                    s.target = None;
                                    s.selected_row = None;
                                    s.action = None;
                                    s.preview = None;
                                    s.task = None;
                                    s.details = None;
                                    s.details_task = None;
                                    s.limit = 200;
                                    d.error = None;
                                    refresh = true;
                                }
                            }
                        });
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let r = search_field(ui, &mut s.filter, crate::i18n::tr("Filter by message or ID"), 240.0);
                            control(widgets, "dialog-reflog-filter", &r, ui.clip_rect());
                        });
                    });
                    if let Some(note) = scope_note(s.scope) {
                        pages::note(ui, note);
                    }
                    if s.scope == ReflogScope::Unreachable && s.log_task.is_some() {
                        let seconds = s.started.map(|t| t.elapsed().as_secs()).unwrap_or(0);
                        let stage = s.scan_stage.clone().unwrap_or_else(|| crate::i18n::tr("Starting").into());
                        pages::callout(
                            ui,
                            BLUE(),
                            &crate::i18n::trf!("Looking through every object in the repository · {seconds} s · {stage}", seconds, stage),
                        );
                    }
                    if let Some(notice) = &s.read_notice {
                        pages::callout(ui, AMBER(), notice);
                    }
                    ui.horizontal_top(|ui| {
                        ui.spacing_mut().item_spacing.x = 0.0;
                        let list_width = (ui.available_width() - PANE - 17.0).max(260.0);
                        ui.vertical(|ui| {
                            ui.set_width(list_width);
                            let list = ScrollArea::vertical()
                                .id_salt("dialog-reflog-list")
                                .max_height(height)
                                .auto_shrink([false, false])
                                .show(ui, |ui| {
                                    ui.set_width(ui.available_width());
                                    entry_list(
                                        ui,
                                        s,
                                        head_target.as_ref(),
                                        &mut d.error,
                                        widgets,
                                    );
                                });
                            pages::region(
                                widgets,
                                "dialog-reflog-list-scroll",
                                list.inner_rect.intersect(ui.clip_rect()),
                                !s.applying,
                            );
                        });
                        ui.add_space(8.0);
                        let (line, _) = ui.allocate_exact_size(vec2(1.0, height), Sense::hover());
                        ui.painter().vline(line.center().x, line.y_range(), Stroke::new(1.0, HAIRLINE()));
                        ui.add_space(8.0);
                        ui.vertical(|ui| {
                            ui.set_width(PANE);
                            let pane = ScrollArea::vertical()
                                .id_salt("dialog-recovery-pane")
                                .max_height(height)
                                .auto_shrink([false, false])
                                .show(ui, |ui| {
                                    ui.set_width(ui.available_width() - 6.0);
                                    ui.spacing_mut().item_spacing.y = 8.0;
                                    let event = match s.action.clone() {
                                        None => details_pane(ui, s, &current, on_branch, head_target.as_ref(), widgets),
                                        Some(RecoveryAction::Branch(_)) => branch_pane(
                                            ui,
                                            s,
                                            &mut d.first,
                                            d.branch_space,
                                            &mut d.error,
                                            &branches,
                                            &current,
                                            idle,
                                            widgets,
                                        ),
                                        Some(RecoveryAction::Reset(_)) => {
                                            reset_pane(ui, s, &d.error, &current, idle, widgets)
                                        }
                                    };
                                    match event {
                                        Event::None => {}
                                        Event::MakeBranch => {
                                            // A deleted branch is offered back under its old name.
                                            let deleted = s
                                                .page
                                                .as_ref()
                                                .filter(|_| s.scope == ReflogScope::DeletedBranches)
                                                .and_then(|p| p.entries.get(s.selected_row?))
                                                .and_then(|e| e.reference.as_ref())
                                                .and_then(|name| String::from_utf8(name.clone()).ok());
                                            if let Some(target) = &s.target {
                                                d.first = deleted.unwrap_or_else(|| {
                                                    format!("recovery/{}", &target.as_str()[..9])
                                                });
                                                s.action = Some(RecoveryAction::Branch(d.first.clone()));
                                                s.edited = None;
                                                d.error = None;
                                                refresh = true;
                                            }
                                        }
                                        Event::Reset => {
                                            s.action = Some(RecoveryAction::Reset(ResetMode::Soft));
                                            d.error = None;
                                            refresh = true;
                                        }
                                        Event::Mode(mode) => {
                                            s.action = Some(RecoveryAction::Reset(mode));
                                            s.task = None;
                                            s.preview = None;
                                            d.error = None;
                                            refresh = true;
                                        }
                                        Event::Back => back = true,
                                        Event::Submit => submit = true,
                                    }
                                });
                            pages::region(
                                widgets,
                                "dialog-recovery-pane-scroll",
                                pane.inner_rect.intersect(ui.clip_rect()),
                                !s.applying,
                            );
                        });
                    });
                }
                pages::Part::Footer if s.applying => {
                    cancel = pages::running_footer(ui, widgets, &running, &[]);
                }
                pages::Part::Footer => {
                    if s.log_task.is_some() || s.task.is_some() {
                        let r = ui.button(crate::i18n::tr("Stop"));
                        control(widgets, "dialog-recovery-cancel-read", &r, ui.clip_rect());
                        cancel_read = r.clicked();
                    }
                    let label = match (browsing, s.scope) {
                        (false, _) => crate::i18n::tr("Review again"),
                        (true, ReflogScope::Unreachable) => crate::i18n::tr("Scan again"),
                        (true, _) => crate::i18n::tr("Refresh"),
                    };
                    let r = ui.add_enabled(!waiting, Button::new(label));
                    control(widgets, "dialog-preview-refresh", &r, ui.clip_rect());
                    refresh |= r.clicked();
                    if browsing {
                        let r = pages::ghost(
                            ui,
                            !waiting && s.limit < 2000 && s.page.as_ref().is_some_and(|p| p.more),
                            crate::i18n::tr("Show older"),
                        );
                        control(widgets, "dialog-reflog-more", &r, ui.clip_rect());
                        if r.clicked() {
                            s.limit = (s.limit + 200).min(2000);
                            if let Some(scan) = &s.lost_scan {
                                match scan.page(s.limit) {
                                    Ok(page) => s.page = Some(page),
                                    Err(e) => d.error = Some(user_error(&e)),
                                }
                            } else {
                                refresh = true;
                            }
                        }
                    }
                    let status = footer_line(s, browsing);
                    pages::footer_status(ui, s.log_task.is_some() || s.task.is_some(), &status);
                }
            },
        );
        // Resizing can place the pointer outside the modal; only the header's × or
        // Escape dismisses a recovery review and its errors.
        dismiss |= (response.close_clicked() || ctx.input(|i| i.key_pressed(Key::Escape)))
            && !s.applying;
        if cancel_read {
            s.log_task = None;
            s.task = None;
            s.progress = None;
            s.preview = None;
            s.scan_stage = None;
            s.read_notice = Some(
                if !browsing {
                    crate::i18n::tr("Review stopped. Review again before making the change.")
                } else if s.page.is_some() {
                    crate::i18n::tr("Scan stopped. Previous completed results are kept.")
                } else {
                    crate::i18n::tr("Scan stopped. No results were published and nothing was changed.")
                }
                .into(),
            );
        }
        if back {
            s.action = None;
            s.preview = None;
            s.task = None;
            s.edited = None;
            d.error = None;
        }
        if cancel {
            self.cancel_write();
        }
        if submit && let (Some(owner), Some(p)) = (d.owner, s.preview.clone()) {
            s.applying = true;
            self.execute(owner, Action::Recovery(p));
            if self.write.is_none() {
                s.applying = false;
                d.error = Some(
                    crate::i18n::tr("Could not start. Wait for the repository, then review again.").into(),
                );
            }
        }
        if !dismiss {
            self.dialog = Some(d);
        }
        if refresh {
            self.start_recovery_read();
        }
        self.start_recovery_details();
    }
}

/// What the details pane asks for.
enum Event {
    None,
    MakeBranch,
    Reset,
    Mode(ResetMode),
    Back,
    Submit,
}

/// The entries, grouped by day, with what each did in words and its commit below.
fn entry_list(
    ui: &mut Ui,
    s: &mut DialogState,
    head_target: Option<&ObjectId>,
    error: &mut Option<String>,
    widgets: &mut HashMap<String, UiControl>,
) {
    let Some(page) = &s.page else {
        if s.log_task.is_some() {
            pages::loading(
                ui,
                if s.scope == ReflogScope::Unreachable { crate::i18n::tr("Scanning") } else { crate::i18n::tr("Reading the history") },
            );
        }
        return;
    };
    if page.entries.is_empty() {
        pages::empty(ui, scope_empty(s.scope));
        return;
    }
    let needle = s.filter.trim().to_lowercase();
    let mut group = None;
    let mut here = false;
    let mut shown = 0;
    let mut chosen = None;
    ui.spacing_mut().item_spacing.y = 2.0;
    for (i, entry) in page.entries.iter().enumerate() {
        let (what, image) = describe(entry, s.scope);
        let subject = entry_subject(entry, s.scope);
        if !needle.is_empty()
            && ![what.as_str(), subject.as_str(), entry.id.as_str()]
                .iter()
                .any(|text| text.to_lowercase().contains(&needle))
        {
            continue;
        }
        shown += 1;
        let (day, time) = when(entry, s.scope);
        if group.as_ref() != Some(&day) {
            ui.add_space(if group.is_none() { 2.0 } else { 8.0 });
            ui.label(RichText::new(&day).size(12.0).color(HINT()).strong());
            group = Some(day);
        }
        let is_here = !here
            && matches!(s.scope, ReflogScope::Head | ReflogScope::AllRefs)
            && head_target == Some(&entry.id);
        here |= is_here;
        let off = s.off.as_ref().is_some_and(|o| o.contains(&entry.id));
        let pill = if is_here {
            Some((crate::i18n::tr("You are here"), BLUE()))
        } else if s.scope == ReflogScope::DeletedBranches {
            Some((crate::i18n::tr("Deleted"), AMBER()))
        } else if off {
            Some((crate::i18n::tr("Not on any branch"), AMBER()))
        } else {
            None
        };
        let r = entry_row(
            ui,
            image,
            &what,
            pill,
            &entry.id.as_str()[..7],
            &subject,
            &time,
            s.selected_row == Some(i),
            off || s.scope == ReflogScope::Unreachable,
            !s.applying,
        )
        .on_hover_text(&subject);
        control(widgets, format!("dialog-reflog-row-{i}"), &r, ui.clip_rect());
        if r.clicked() {
            chosen = Some((i, entry.id.clone()));
        }
    }
    if shown == 0 {
        pages::empty(ui, crate::i18n::tr("Nothing matches the filter."));
    }
    if page.more && s.limit == 2000 {
        ui.add_space(6.0);
        pages::note(ui, crate::i18n::tr("Showing the latest 2,000 entries."));
    }
    if let Some((i, id)) = chosen {
        s.show_diff &= s.target.as_ref() == Some(&id);
        s.target = Some(id);
        s.selected_row = Some(i);
        s.action = None;
        s.preview = None;
        s.task = None;
        s.edited = None;
        *error = None;
    }
}

/// One entry: an icon, what happened and a pill, then the commit; the time right.
#[allow(clippy::too_many_arguments)]
fn entry_row(
    ui: &mut Ui,
    image: Icon,
    what: &str,
    pill: Option<(&str, Color32)>,
    sha: &str,
    subject: &str,
    time: &str,
    selected: bool,
    lost: bool,
    enabled: bool,
) -> Response {
    let (rect, response) = ui
        .add_enabled_ui(enabled, |ui| {
            ui.allocate_exact_size(vec2(ui.available_width(), 46.0), Sense::click())
        })
        .inner;
    let painter = ui.painter();
    if selected {
        painter.rect_filled(rect, 8.0, SELECTED());
        painter.rect_filled(
            Rect::from_min_size(rect.min + vec2(0.0, 11.0), vec2(3.0, rect.height() - 22.0)),
            1.5,
            BLUE(),
        );
    } else if enabled && (response.hovered() || response.has_focus()) {
        painter.rect_filled(rect, 8.0, HOVER());
    }
    let tile = Rect::from_min_size(pos2(rect.left() + 10.0, rect.center().y - 15.0), vec2(30.0, 30.0));
    painter.rect_filled(tile, 7.0, CARD());
    icon(
        painter,
        tile.shrink(8.0),
        image,
        if selected { BLUE() } else if lost { AMBER() } else { HINT() },
    );
    let right = rect.right() - 10.0;
    let time = painter.layout_no_wrap(time.to_owned(), FontId::proportional(12.0), HINT());
    let time_width = time.size().x;
    painter.galley(pos2(right - time_width, rect.center().y - time.size().y / 2.0), time, HINT());
    let left = tile.right() + 12.0;
    let width = (right - time_width - 12.0 - left).max(40.0);
    let pill = pill.map(|(text, color)| {
        (painter.layout_no_wrap(text.to_owned(), FontId::proportional(11.5), color), color)
    });
    let pill_width = pill.as_ref().map_or(0.0, |(g, _)| g.size().x + 14.0);
    let mut job = text::LayoutJob::simple_singleline(what.to_owned(), semibold(13.0), TEXT());
    job.wrap = text::TextWrapping::truncate_at_width((width - pill_width - 8.0).max(20.0));
    let title = painter.layout_job(job);
    let line = rect.top() + 14.0;
    let title_width = title.size().x;
    painter.galley(pos2(left, line - title.size().y / 2.0), title, TEXT());
    if let Some((galley, color)) = pill {
        let pill_rect = Rect::from_min_size(pos2(left + title_width + 8.0, line - 8.5), vec2(pill_width, 17.0));
        painter.rect_filled(pill_rect, 8.5, color.gamma_multiply(0.14));
        painter.galley(
            pos2(pill_rect.left() + 7.0, pill_rect.center().y - galley.size().y / 2.0),
            galley,
            color,
        );
    }
    let mut job = text::LayoutJob::default();
    job.append(sha, 0.0, TextFormat::simple(FontId::monospace(11.5), MUTED()));
    job.append(&format!(" · {subject}"), 0.0, TextFormat::simple(FontId::proportional(12.0), HINT()));
    job.wrap = text::TextWrapping::truncate_at_width(width);
    let detail = painter.layout_job(job);
    painter.galley(pos2(left, rect.bottom() - 13.0 - detail.size().y / 2.0), detail, HINT());
    response.widget_info(|| WidgetInfo::selected(WidgetType::Button, enabled, selected, what));
    response
}

/// The files of a patch with their added and removed lines, at most `limit`.
fn patch_files(ui: &mut Ui, patch: &[u8], limit: usize) {
    let parsed = crate::diff::Patch::parse(patch);
    for f in parsed.files.iter().take(limit) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let counts = format!("+{} −{}", f.added, f.removed);
            let width = ui.painter().layout_no_wrap(counts.clone(), FontId::proportional(12.0), HINT()).size().x;
            pages::left_label(
                ui,
                (ui.available_width() - width - 8.0).max(40.0),
                Label::new(RichText::new(&f.label).size(12.5)),
            );
            ui.label(RichText::new(counts).size(12.0).color(HINT()));
        });
    }
    if parsed.files.len() > limit {
        pages::note(ui, &crate::i18n::trf!("More: {count}", count = parsed.files.len() - limit));
    }
}

/// The patch as text, bounded.
fn patch_text(ui: &mut Ui, patch: &[u8]) {
    let text = String::from_utf8_lossy(patch);
    ScrollArea::vertical()
        .id_salt("dialog-recovery-patch")
        .max_height(220.0)
        .show(ui, |ui| {
            pages::well(ui, |ui| {
                ui.add(Label::new(RichText::new(text.chars().take(16000).collect::<String>()).monospace().size(12.0)).wrap());
                if text.chars().count() > 16000 {
                    pages::note(ui, crate::i18n::tr("Showing the first 16,000 characters."));
                }
            });
        });
}

/// "1 commit ahead", "3 behind", "2 ahead, 1 behind", "the same commit".
fn relation_text(ahead: u64, behind: u64) -> String {
    match (ahead, behind) {
        (0, 0) => crate::i18n::tr("The same commit").into(),
        (a, 0) => crate::i18n::trf!("Commits ahead: {count}", count = a),
        (0, b) => crate::i18n::trf!("Commits behind: {count}", count = b),
        (a, b) => crate::i18n::trf!("Commits ahead: {ahead} · behind: {behind}", ahead = a, behind = b),
    }
}

/// The selected entry: its commit, whether a branch has it, how it compares with the
/// current branch, and what you can do.
fn details_pane(
    ui: &mut Ui,
    s: &mut DialogState,
    current: &str,
    on_branch: bool,
    head_target: Option<&ObjectId>,
    widgets: &mut HashMap<String, UiControl>,
) -> Event {
    let Some(target) = s.target.clone() else {
        ui.add_space(30.0);
        ui.vertical_centered(|ui| {
            ui.label(RichText::new(crate::i18n::tr("Select an entry to see its commit and what you can do with it.")).color(MUTED()));
        });
        return Event::None;
    };
    let entry = s.page.as_ref().and_then(|p| p.entries.get(s.selected_row?)).cloned();
    let lost = s.scope == ReflogScope::Unreachable;
    let scanned = s
        .lost_scan
        .as_ref()
        .and_then(|scan| scan.commits().iter().find(|c| c.id == target).cloned());
    let details = match &s.details {
        Some((id, Ok(details))) if *id == target => Some(details.clone()),
        _ => None,
    };
    let commit = details.as_ref().map(|d| d.commit.clone()).or(scanned);
    let subject = commit
        .as_ref()
        .map(subject)
        .or_else(|| entry.as_ref().map(|e| entry_subject(e, s.scope)))
        .unwrap_or_default();
    ui.add(Label::new(RichText::new(&subject).font(semibold(16.0))).wrap());
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        ui.label(RichText::new(&target.as_str()[..7]).monospace().size(12.0).color(MUTED()));
        if let Some(c) = &commit {
            ui.label(
                RichText::new(format!(" · {} · {}", display(&c.author.name), date(c.author.timestamp, c.author.offset_minutes)))
                    .size(12.0)
                    .color(HINT()),
            );
        }
        let r = crate::chrome::icon_button(ui, Icon::Copy, true, false, 22.0).on_hover_text(crate::i18n::tr("Copy the full commit ID"));
        control(widgets, "dialog-recovery-copy", &r, ui.clip_rect());
        if r.clicked() {
            ui.ctx().copy_text(target.to_string());
        }
    });
    let off = s.off.as_ref().is_some_and(|o| o.contains(&target));
    let (color, state) = if lost {
        (AMBER(), crate::i18n::tr("Nothing reaches this commit, so Git deletes it at its next cleanup. Make a branch to keep it.").to_owned())
    } else if head_target == Some(&target) {
        (BLUE(), crate::i18n::trf!("{current} is here now.", current))
    } else if off {
        (AMBER(), crate::i18n::tr("No branch includes this commit. Make a branch to keep it.").to_owned())
    } else {
        (BLUE(), crate::i18n::tr("A branch already includes this commit.").to_owned())
    };
    pages::callout(ui, color, &state);
    if lost {
        crate::tags::card(ui, crate::i18n::tr("Found by the scan"), |ui| {
            if let Some(c) = &commit {
                pages::key_value(ui, crate::i18n::tr("Parents"), 96.0, |ui| {
                    ui.label(match c.parents.len() {
                        0 => crate::i18n::tr("None: a root commit").to_owned(),
                        1 => c.parents[0].as_str()[..7].to_owned(),
                        n => crate::i18n::trf!("{count}, a merge", count = n),
                    });
                });
            }
            pages::note(ui, crate::i18n::tr("Making a branch shows the change it holds first."));
        });
    } else {
        match &s.details {
            Some((id, Err(error))) if *id == target => pages::callout(ui, AMBER(), error),
            _ if details.is_none() => pages::loading(ui, crate::i18n::tr("Reading the commit")),
            _ => {}
        }
        if let Some(details) = &details {
            crate::tags::card(ui, &crate::i18n::trf!("Compared with {current}", current), |ui| {
                let files = crate::diff::Patch::parse(&details.patch).files.len();
                let line = match details.relation {
                    Some((ahead, behind)) => format!(
                        "{} · {}",
                        relation_text(ahead, behind),
                        crate::i18n::trf!("Files that differ: {count}", count = files)
                    ),
                    None => crate::i18n::tr("There is no current commit to compare with.").into(),
                };
                ui.label(RichText::new(line).color(MUTED()));
                patch_files(ui, &details.patch, 6);
                if !details.patch.is_empty() {
                    let r = pages::disclosure(ui, s.show_diff, crate::i18n::tr("Show diff"));
                    control(widgets, "dialog-recovery-diff", &r, ui.clip_rect());
                    if r.clicked() {
                        s.show_diff = !s.show_diff;
                    }
                    if s.show_diff {
                        patch_text(ui, &details.patch);
                    }
                }
            });
        }
    }
    ui.add_space(4.0);
    pages::divider(ui);
    let mut event = Event::None;
    let full = vec2(ui.available_width(), 32.0);
    let r = ui.add_enabled(
        !s.applying,
        Button::new(RichText::new(crate::i18n::tr("Make a branch here")).color(ACCENT_INK()).strong())
            .fill(BLUE())
            .min_size(full),
    );
    control(widgets, "dialog-recovery-branch", &r, ui.clip_rect());
    if r.clicked() {
        event = Event::MakeBranch;
    }
    if !lost && on_branch {
        let r = ui.add_enabled(
            !s.applying && head_target != Some(&target),
            Button::new(crate::i18n::trf!("Move {current} here", current)).min_size(full),
        );
        control(widgets, "dialog-recovery-reset", &r, ui.clip_rect());
        if r.clicked() {
            event = Event::Reset;
        }
    }
    event
}

/// A link back to the selected entry, above an action's form.
fn back_link(ui: &mut Ui, s: &DialogState, widgets: &mut HashMap<String, UiControl>) -> bool {
    let label = s
        .preview
        .as_ref()
        .map(|p| subject(p.target()))
        .or_else(|| {
            let entry = s.page.as_ref()?.entries.get(s.selected_row?)?;
            Some(entry_subject(entry, s.scope))
        })
        .unwrap_or_else(|| crate::i18n::tr("Back").into());
    let r = pages::tool(ui, Icon::ArrowLeft, &label, !s.applying);
    control(widgets, "dialog-recovery-back", &r, ui.clip_rect());
    r.clicked()
}

/// Make a branch at the selected commit: its name, then what the review found.
#[allow(clippy::too_many_arguments)]
fn branch_pane(
    ui: &mut Ui,
    s: &mut DialogState,
    name: &mut String,
    space: crate::general::BranchSpace,
    error: &mut Option<String>,
    branches: &[Branch],
    current: &str,
    idle: bool,
    widgets: &mut HashMap<String, UiControl>,
) -> Event {
    let mut event = Event::None;
    if back_link(ui, s, widgets) {
        event = Event::Back;
    }
    let sha = s.target.as_ref().map_or("", |t| &t.as_str()[..7]).to_owned();
    ui.label(RichText::new(crate::i18n::trf!("Make a branch at {sha}", sha)).font(semibold(16.0)));
    ui.label(RichText::new(crate::i18n::tr("Branch name")).size(12.0).color(MUTED()));
    let r = ui.add_enabled(
        !s.applying,
        TextEdit::singleline(name)
            .desired_width(f32::INFINITY)
            .min_size(vec2(0.0, 30.0))
            .vertical_align(Align::Center),
    );
    control(widgets, "dialog-recovery-name", &r, ui.clip_rect());
    if r.changed() {
        space.apply(name);
        s.task = None;
        s.preview = None;
        *error = None;
        s.edited = Some(Instant::now());
    }
    let trimmed = name.trim();
    if !trimmed.is_empty() {
        match crate::small_dialogs::branch_name_problem(trimmed, branches, None) {
            Some(problem) => check(ui, false, &problem),
            None if s.preview.is_some() => check(ui, true, crate::i18n::tr("Available")),
            None => {}
        }
    }
    if let Some(error) = error.as_ref() {
        pages::callout(ui, RED(), error);
    }
    if s.task.is_some() {
        pages::waiting(ui, crate::i18n::tr("Reviewing"));
    }
    if let Some(p) = &s.preview {
        let text = crate::i18n::trf!(
            "The new branch starts at {sha} · {subject}. You stay on {current} and no files change.",
            sha, subject = subject(p.target()), current
        );
        pages::callout(ui, BLUE(), &text);
        if p.is_lost_commit() {
            crate::tags::card(ui, crate::i18n::tr("The change it holds"), |ui| {
                patch_files(ui, p.patch(), 8);
            });
        }
    }
    pages::divider(ui);
    ui.horizontal(|ui| {
        let r = ui.add_enabled(!s.applying, Button::new(crate::i18n::tr("Back")).min_size(vec2(72.0, 32.0)));
        control(widgets, "dialog-recovery-back-button", &r, ui.clip_rect());
        if r.clicked() {
            event = Event::Back;
        }
        let ready = idle && !s.applying && s.task.is_none() && s.preview.is_some() && error.is_none();
        let r = ui.add_enabled(
            ready,
            Button::new(RichText::new(crate::i18n::tr("Make branch")).color(ACCENT_INK()).strong())
                .fill(BLUE())
                .min_size(vec2(ui.available_width(), 32.0)),
        );
        control(widgets, "dialog-submit", &r, ui.clip_rect());
        if r.clicked() {
            event = Event::Submit;
        }
    });
    event
}

/// Move the current branch to the selected commit: what happens to your changes,
/// then what the review found.
fn reset_pane(
    ui: &mut Ui,
    s: &mut DialogState,
    error: &Option<String>,
    current: &str,
    idle: bool,
    widgets: &mut HashMap<String, UiControl>,
) -> Event {
    let mut event = Event::None;
    if back_link(ui, s, widgets) {
        event = Event::Back;
    }
    let sha = s.target.as_ref().map_or("", |t| &t.as_str()[..7]).to_owned();
    ui.label(RichText::new(crate::i18n::trf!("Move {current} to {sha}", current, sha)).font(semibold(16.0)));
    let original = s.preview.as_ref().and_then(|p| match p.head() {
        Head::Branch { target, .. } => Some(target.as_str()[..7].to_owned()),
        _ => None,
    });
    move_picture(ui, original.as_deref().unwrap_or(crate::i18n::tr("now")), &sha);
    ui.label(RichText::new(crate::i18n::tr("Your current changes")).size(12.0).color(MUTED()));
    let mode = match s.action {
        Some(RecoveryAction::Reset(mode)) => mode,
        _ => ResetMode::Soft,
    };
    let hard_detail = crate::i18n::trf!("Files become exactly {sha}. Only when nothing is changed.", sha);
    let choices = [
        (ResetMode::Soft, "soft", crate::i18n::tr("Keep them staged"), crate::i18n::tr("Only the branch moves.")),
        (ResetMode::Mixed, "mixed", crate::i18n::tr("Keep them unstaged"), crate::i18n::tr("The branch moves and staging is cleared. Files stay.")),
        (ResetMode::Hard, "hard", crate::i18n::tr("Replace them"), hard_detail.as_str()),
    ];
    for (value, id, name, about) in choices {
        let r = pages::option_cards(ui, !s.applying, &[(mode == value, name, about)]);
        control(widgets, format!("dialog-reset-{id}"), &r[0], ui.clip_rect());
        if r[0].clicked() && mode != value {
            event = Event::Mode(value);
        }
    }
    if let Some(error) = error {
        pages::callout(ui, RED(), error);
    }
    if s.task.is_some() {
        pages::waiting(ui, crate::i18n::tr("Reviewing"));
    }
    if let Some(p) = &s.preview
        && p.recovery_reference().is_some()
    {
        let kept = original.as_deref().unwrap_or(crate::i18n::tr("the current commit"));
        pages::callout(ui, BLUE(), &crate::i18n::trf!("Rocket keeps {kept} under Kept by Rocket, so you can move back from here.", kept));
    }
    pages::divider(ui);
    ui.horizontal(|ui| {
        let r = ui.add_enabled(!s.applying, Button::new(crate::i18n::tr("Back")).min_size(vec2(72.0, 32.0)));
        control(widgets, "dialog-recovery-back-button", &r, ui.clip_rect());
        if r.clicked() {
            event = Event::Back;
        }
        let ready = idle && !s.applying && s.task.is_none() && s.preview.is_some() && error.is_none();
        let (fill, ink) = if mode == ResetMode::Hard {
            (RED(), RED_INK())
        } else {
            (BLUE(), ACCENT_INK())
        };
        let r = ui.add_enabled(
            ready,
            Button::new(RichText::new(crate::i18n::tr("Move branch")).color(ink).strong())
                .fill(fill)
                .min_size(vec2(ui.available_width(), 32.0)),
        );
        control(widgets, "dialog-submit", &r, ui.clip_rect());
        if r.clicked() {
            event = Event::Submit;
        }
    });
    event
}

/// The branch's tip now and after the move.
fn move_picture(ui: &mut Ui, now: &str, after: &str) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 58.0), Sense::hover());
    let painter = ui.painter_at(rect);
    let y = rect.top() + 24.0;
    let (a, b) = (rect.left() + rect.width() * 0.28, rect.left() + rect.width() * 0.72);
    painter.line_segment([pos2(rect.left() + 8.0, y), pos2(rect.right() - 8.0, y)], Stroke::new(2.0, WINDOW_EDGE()));
    painter.circle_filled(pos2(a, y), 6.0, MUTED());
    painter.circle_filled(pos2(b, y), 6.0, BLUE());
    let arc = Shape::dashed_line(
        &[pos2(a + 10.0, y - 10.0), pos2((a + b) / 2.0, y - 20.0), pos2(b - 10.0, y - 10.0)],
        Stroke::new(1.5, BLUE()),
        4.0,
        3.0,
    );
    painter.extend(arc);
    painter.text(pos2(a, y + 12.0), Align2::CENTER_TOP, crate::i18n::trf!("now · {commit}", commit = now), FontId::proportional(11.5), HINT());
    painter.text(pos2(b, y + 12.0), Align2::CENTER_TOP, crate::i18n::trf!("after · {commit}", commit = after), FontId::proportional(11.5), TEXT());
}

/// The footer line: how many entries, or where a review stands.
fn footer_line(s: &DialogState, browsing: bool) -> String {
    if !browsing {
        return if s.task.is_some() {
            crate::i18n::tr("Reviewing").into()
        } else if s.preview.is_some() {
            crate::i18n::tr("Reviewed · nothing has changed yet").into()
        } else {
            crate::i18n::tr("Review to see what will happen").into()
        };
    }
    if s.log_task.is_some() {
        return if s.scope == ReflogScope::Unreachable { crate::i18n::tr("Scanning").into() } else { crate::i18n::tr("Reading").into() };
    }
    match &s.page {
        Some(page) => {
            let shown = match (s.scope == ReflogScope::Unreachable, page.entries.len()) {
                (true, n) => crate::i18n::trf!("Lost commits found: {count}", count = n),
                (false, n) => crate::i18n::trf!("Entries: {count} · newest first", count = n),
            };
            if page.more {
                crate::i18n::trf!("{shown} · older ones with Show older", shown)
            } else {
                shown
            }
        }
        None => String::new(),
    }
}

#[cfg(test)]
mod flow_tests {
    use crate::{
        app::{DialogKind, Intent},
        batch::flow_tests::{click, open, wait},
        test_common as common,
    };
    use eframe::egui::Context;

    /// A commit left behind by a reset is marked, shows how it compares, and gets a
    /// branch from the details pane.
    #[test]
    fn a_lost_commit_shows_its_details_and_gets_a_branch() {
        common::isolated(
            "ROCKET_RECOVERY_UI_CASE",
            "recovery::flow_tests::a_lost_commit_shows_its_details_and_gets_a_branch",
            || {
                let f = common::Fixture::new();
                f.write("a.txt", "1\n");
                f.commit("base");
                f.write("a.txt", "2\n");
                f.commit("PRIVATE_lost");
                let rev = |name: &str| String::from_utf8(f.run(&["rev-parse", name])).unwrap().trim().to_owned();
                let lost = rev("HEAD");
                f.run(&["reset", "--quiet", "--hard", "HEAD~1"]);
                let ctx = Context::default();
                let mut app = open(&ctx, &f);
                app.dispatch(Intent::Dialog(DialogKind::Recovery));
                wait(&ctx, &mut app);
                // Newest first: the reset, then the commit it left behind.
                let s = app.dialog.as_ref().unwrap().recovery.as_ref().unwrap();
                let entry = &s.page.as_ref().unwrap().entries[1];
                assert_eq!(entry.id.as_str(), lost);
                assert!(s.off.as_ref().unwrap().contains(&entry.id), "no branch has it");
                click(&ctx, &mut app, "dialog-reflog-row-1");
                wait(&ctx, &mut app);
                let s = app.dialog.as_ref().unwrap().recovery.as_ref().unwrap();
                let (id, details) = s.details.as_ref().unwrap();
                assert_eq!(id.as_str(), lost);
                assert_eq!(details.as_ref().unwrap().relation, Some((1, 0)));
                click(&ctx, &mut app, "dialog-recovery-branch");
                wait(&ctx, &mut app);
                let name = format!("recovery/{}", &lost[..9]);
                assert_eq!(app.dialog.as_ref().unwrap().first, name);
                click(&ctx, &mut app, "dialog-submit");
                wait(&ctx, &mut app);
                assert_eq!(rev(&name), lost);
                assert!(app.dialog.is_none(), "it closes once the branch exists");
            },
        );
    }

    /// Refresh reads the selected commit again: the branch may have moved since.
    #[test]
    fn refresh_reads_the_selected_commit_again() {
        common::isolated(
            "ROCKET_RECOVERY_UI_CASE",
            "recovery::flow_tests::refresh_reads_the_selected_commit_again",
            || {
                let f = common::Fixture::new();
                f.write("a.txt", "1\n");
                f.commit("base");
                f.write("a.txt", "2\n");
                f.commit("PRIVATE_lost");
                f.run(&["reset", "--quiet", "--hard", "HEAD~1"]);
                let ctx = Context::default();
                let mut app = open(&ctx, &f);
                app.dispatch(Intent::Dialog(DialogKind::Recovery));
                wait(&ctx, &mut app);
                click(&ctx, &mut app, "dialog-reflog-row-1");
                wait(&ctx, &mut app);
                let relation = |app: &crate::app::Rocket| {
                    let s = app.dialog.as_ref().unwrap().recovery.as_ref().unwrap();
                    s.details.as_ref().unwrap().1.as_ref().unwrap().relation
                };
                assert_eq!(relation(&app), Some((1, 0)));
                // The branch moves on outside Rocket.
                f.write("b.txt", "1\n");
                f.commit("PRIVATE_later");
                click(&ctx, &mut app, "dialog-preview-refresh");
                wait(&ctx, &mut app);
                assert_eq!(relation(&app), Some((1, 1)));
            },
        );
    }
}
