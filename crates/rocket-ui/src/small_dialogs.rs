//! The small dialogs `menus.rs` draws with its shared renderer: new, rename and
//! delete branches, add and edit remotes, fetch, push, save stash, the repository
//! identity, the confirmations, close and exit. Fields check as you type, what
//! will happen sits in one card, and the footer says where things stand.
use crate::{
    app::*,
    backend::{Action, Backend, user_error},
    pages::{self, control},
    style::*,
    tags::{age, card, check, commit_ref},
};
use eframe::egui::*;
use rocket_git::*;
use std::{
    collections::HashMap,
    path::Path,
    time::{Duration, Instant},
};

/// How long Push waits after an edit before it reads what the push would send.
const SETTLE: Duration = Duration::from_millis(400);
/// Commits the Push card lists before "and N more".
const LISTED: usize = 5;
/// The label column of the dialogs' cards.
const KEY: f32 = 104.0;

/// The small dialogs' own reads.
#[derive(Default)]
pub struct State {
    merge_task: Option<ReadTask<Vec<BranchMergeState>>>,
    /// What Git's ordinary branch deletion would accept, once read.
    pub merge: Option<Vec<BranchMergeState>>,
    merge_error: Option<String>,
    outlook_task: Option<(Target, ReadTask<PushOutlook>)>,
    outlook: Option<(Target, std::result::Result<PushOutlook, String>)>,
    /// The push target as last edited, and when, until the read starts.
    typed: Option<(Target, Instant)>,
    /// A button asked for another dialog in this one's place, after this frame.
    pub open: Option<DialogKind>,
}

/// Push: the remote, the local branch and the destination as typed.
type Target = (Vec<u8>, Vec<u8>, String);

impl State {
    /// A read is running.
    pub(crate) fn running(&self) -> bool {
        self.merge_task.is_some() || self.outlook_task.is_some()
    }

    /// Take finished reads; keep repainting while one runs or an edit settles.
    pub(crate) fn poll(&mut self, ctx: &Context) {
        if let Some(result) = self.merge_task.as_mut().and_then(ReadTask::try_finish) {
            self.merge_task = None;
            match result {
                Ok(states) => self.merge = Some(states),
                Err(error) => self.merge_error = Some(user_error(&error)),
            }
        }
        let finished = match &mut self.outlook_task {
            Some((target, task)) => task.try_finish().map(|r| (target.clone(), r)),
            None => None,
        };
        if let Some((target, result)) = finished {
            self.outlook_task = None;
            self.outlook = Some((target, result.map_err(|e| user_error(&e))));
        }
        if self.merge_task.is_some() || self.outlook_task.is_some() || self.typed.is_some() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }

    /// Read once per dialog whether Git would delete each of `names`.
    fn read_merge(&mut self, backend: Option<&Backend>, root: Option<&Path>, names: &[Vec<u8>]) {
        if self.merge.is_some() || self.merge_task.is_some() || self.merge_error.is_some() {
            return;
        }
        let (Some(backend), Some(root)) = (backend.cloned(), root.map(Path::to_path_buf)) else {
            return;
        };
        let names = names.to_vec();
        match ReadTask::spawn(move |cancel| backend.branch_merge_states(&root, &names, &cancel)) {
            Ok(task) => self.merge_task = Some(task),
            Err(error) => self.merge_error = Some(user_error(&error)),
        }
    }

    fn merge_of(&self, name: &[u8]) -> Option<&BranchMergeState> {
        self.merge.as_ref()?.iter().find(|s| s.name == name)
    }

    /// Read what pushing `target` would send: at once the first time, then after
    /// typing pauses. An older read is dropped, which cancels it.
    fn read_outlook(&mut self, target: Target, backend: Option<&Backend>, root: Option<&Path>) {
        let same = |t: &Target| *t == target;
        if self.outlook.as_ref().is_some_and(|(t, _)| same(t))
            || self.outlook_task.as_ref().is_some_and(|(t, _)| same(t))
        {
            self.typed = None;
            return;
        }
        let settled = match &self.typed {
            Some((t, at)) if same(t) => at.elapsed() >= SETTLE,
            _ => {
                let first = self.outlook.is_none() && self.outlook_task.is_none();
                self.typed = Some((target.clone(), Instant::now()));
                first
            }
        };
        if !settled {
            return;
        }
        self.typed = None;
        self.outlook_task = None;
        let destination = target.2.trim().to_owned();
        let (Some(backend), Some(root)) = (backend.cloned(), root.map(Path::to_path_buf)) else {
            return;
        };
        if target.0.is_empty() || target.1.is_empty() || destination.is_empty() {
            self.outlook = None;
            return;
        }
        let (remote, branch) = (target.0.clone(), target.1.clone());
        match ReadTask::spawn(move |cancel| {
            backend.push_outlook(&root, &branch, &remote, destination.as_bytes(), &cancel)
        }) {
            Ok(task) => self.outlook_task = Some((target, task)),
            Err(error) => self.outlook = Some((target, Err(user_error(&error)))),
        }
    }

    /// The read for `target`, if it has finished.
    fn outlook_for(&self, target: &Target) -> Option<&std::result::Result<PushOutlook, String>> {
        self.outlook.as_ref().filter(|(t, _)| t == target).map(|(_, r)| r)
    }
}

/// A caption above a field.
fn label(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).size(12.0).color(MUTED()));
}

/// A 30-point one-line field with a faint placeholder.
fn line(ui: &mut Ui, enabled: bool, text: &mut String, hint: &str) -> Response {
    ui.add_enabled(
        enabled,
        TextEdit::singleline(text)
            .hint_text(RichText::new(hint).color(FAINT()))
            .char_limit(4096)
            .desired_width(f32::INFINITY)
            .min_size(vec2(0.0, 30.0))
            .vertical_align(Align::Center),
    )
}

/// A plain value in a card row.
fn value(ui: &mut Ui, text: &str) {
    ui.add(Label::new(text).truncate());
}

/// A value and a dimmed tail on one line, cut short rather than wrapped.
fn value_tail(ui: &mut Ui, text: &str, tail: &str) {
    let mut job = text::LayoutJob::default();
    job.append(text, 0.0, TextFormat::simple(FontId::proportional(13.0), TEXT()));
    job.append(tail, 0.0, TextFormat::simple(FontId::proportional(13.0), FAINT()));
    job.wrap = text::TextWrapping::truncate_at_width(ui.available_width());
    ui.label(job);
}

fn snapshot(tab: Option<&Tab>) -> Option<&crate::backend::Snapshot> {
    tab.and_then(|t| t.snapshot.as_ref())
}

fn branches(tab: Option<&Tab>) -> &[Branch] {
    snapshot(tab).map_or(&[], |s| s.branches.as_slice())
}

/// A loaded history commit, for its subject, author and age.
fn commit_in<'a>(tab: Option<&'a Tab>, id: &ObjectId) -> Option<&'a Commit> {
    tab?.loaded_commits().iter().find(|c| c.id == *id)
}

/// The current branch's name, or "Detached HEAD".
fn current(tab: Option<&Tab>) -> String {
    head_name(snapshot(tab).and_then(|s| s.head.as_ref()))
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// Why Git would refuse `name` for a local branch, in words; None when it looks
/// valid. `old` is the branch being renamed. Git checks again when it writes.
pub(crate) fn branch_name_problem(
    name: &str,
    branches: &[Branch],
    old: Option<&[u8]>,
) -> Option<String> {
    if old.is_some_and(|old| old == name.as_bytes()) {
        return Some("That is its current name.".into());
    }
    let problem = if name.starts_with("refs/") {
        "Leave out refs/heads/: enter just the name."
    } else if name.chars().any(char::is_whitespace) {
        let joined = name.split_whitespace().collect::<Vec<_>>().join("-");
        return Some(format!("Branch names can't contain spaces. Try {joined}."));
    } else if name.starts_with('-') {
        "A branch name can't start with a dash."
    } else if let Some(c) = name.chars().find(|c| c.is_control() || "~^:?*[\\".contains(*c)) {
        return Some(if c.is_control() {
            "Branch names can't contain control characters.".into()
        } else {
            format!("Branch names can't contain {c}")
        });
    } else if name.contains("..") {
        "Branch names can't contain two dots in a row."
    } else if name.contains("@{") || name == "@" || name == "HEAD" {
        "That name is reserved by Git."
    } else if name.starts_with('/') || name.ends_with('/') || name.contains("//") {
        "Use single slashes between parts, not at the start or end."
    } else if name.ends_with('.') {
        "A branch name can't end with a dot."
    } else if name.split('/').any(|part| part.starts_with('.') || part.ends_with(".lock")) {
        "No part of a branch name can start with a dot or end with .lock."
    } else {
        let others = branches
            .iter()
            .filter(|b| b.kind == BranchKind::Local)
            .filter_map(|b| b.reference.strip_prefix(b"refs/heads/"))
            .filter(|n| old != Some(*n));
        for existing in others {
            if existing == name.as_bytes() {
                return Some("A branch with this name already exists.".into());
            }
            let existing = display(existing);
            if existing.eq_ignore_ascii_case(name) {
                return Some(format!(
                    "Too close to {existing}: names that differ only in letter case clash on Windows."
                ));
            }
            if existing.starts_with(&format!("{name}/")) || name.starts_with(&format!("{existing}/")) {
                return Some(format!(
                    "Clashes with {existing}: a name can't be both a branch and a folder of branches."
                ));
            }
        }
        return None;
    };
    Some(problem.into())
}

/// New branch and Rename branch can go ahead; a tracking branch also needs its
/// remote branch to still be there.
pub(crate) fn branch_name_ready(dialog: &Dialog, tab: Option<&Tab>) -> bool {
    let name = dialog.first.trim();
    let old = match &dialog.kind {
        DialogKind::Rename(old) => Some(old.as_slice()),
        _ => None,
    };
    let start = match &dialog.kind {
        DialogKind::TrackBranch { upstream, .. } => branches(tab).iter().any(|b| b.reference == *upstream),
        _ => true,
    };
    start && !name.is_empty() && branch_name_problem(name, branches(tab), old).is_none()
}

/// New branch and Rename branch: the name with its live check, then where the new
/// branch starts or what the rename changes.
pub(crate) fn branch_name_body(
    ui: &mut Ui,
    dialog: &mut Dialog,
    tab: Option<&Tab>,
    widgets: &mut HashMap<String, UiControl>,
) {
    let old = match &dialog.kind {
        DialogKind::Rename(old) => Some(old.clone()),
        _ => None,
    };
    label(ui, if old.is_some() { "New name" } else { "Name" });
    let r = line(ui, true, &mut dialog.first, "feature/my-change");
    control(widgets, "dialog-first", &r, ui.clip_rect());
    if r.changed() {
        dialog.error = None;
        dialog.branch_space.apply(&mut dialog.first);
    }
    let name = dialog.first.trim().to_owned();
    if !name.is_empty() {
        match branch_name_problem(&name, branches(tab), old.as_deref()) {
            Some(problem) => check(ui, false, &problem),
            None => check(ui, true, "Available"),
        }
    }
    ui.add_space(4.0);
    if let DialogKind::TrackBranch { upstream, .. } = &dialog.kind {
        card(ui, "Starts at", |ui| {
            match branches(tab).iter().find(|b| b.reference == *upstream) {
                Some(b) => {
                    let commit = commit_in(tab, &b.target);
                    pages::key_value(ui, "Commit", KEY, |ui| commit_ref(ui, &b.target, commit, ""));
                    if let Some(c) = commit {
                        pages::key_value(ui, "Made by", KEY, |ui| {
                            value_tail(ui, &display(&c.author.name), &format!(" · {}", age(c.author.timestamp)));
                        });
                    }
                }
                None => pages::note(ui, "This remote branch is gone. Fetch, then try again."),
            }
            pages::key_value(ui, "Tracks", KEY, |ui| value(ui, &short_ref(upstream)));
            pages::key_value(ui, "Then", KEY, |ui| value(ui, "You switch to it"));
        });
        return;
    }
    match old {
        None => card(ui, "Starts at", |ui| match snapshot(tab).and_then(|s| s.head.as_ref()) {
            Some(Head::Branch { name, target }) => {
                let commit = commit_in(tab, target);
                pages::key_value(ui, "Commit", KEY, |ui| commit_ref(ui, target, commit, ""));
                if let Some(c) = commit {
                    pages::key_value(ui, "Made by", KEY, |ui| {
                        value_tail(ui, &display(&c.author.name), &format!(" · {}", age(c.author.timestamp)));
                    });
                }
                pages::key_value(ui, "You stay on", KEY, |ui| value(ui, &display(name)));
            }
            Some(Head::Detached { target }) => {
                pages::key_value(ui, "Commit", KEY, |ui| {
                    commit_ref(ui, target, commit_in(tab, target), "")
                });
                pages::key_value(ui, "You stay on", KEY, |ui| value(ui, "Detached HEAD"));
            }
            _ => pages::note(ui, "This repository has no commits yet."),
        }),
        Some(old) => card(ui, "What changes", |ui| {
            let mut full = b"refs/heads/".to_vec();
            full.extend_from_slice(&old);
            let branch = branches(tab).iter().find(|b| b.reference == full);
            let valid = !name.is_empty()
                && branch_name_problem(&name, branches(tab), Some(old.as_slice())).is_none();
            pages::key_value(ui, "Branch", KEY, |ui| {
                value_tail(ui, &display(&old), &if valid { format!(" → {name}") } else { String::new() });
            });
            if let Some(b) = branch {
                pages::key_value(ui, "Points to", KEY, |ui| {
                    commit_ref(ui, &b.target, commit_in(tab, &b.target), "")
                });
                pages::key_value(ui, "On a remote", KEY, |ui| match &b.upstream {
                    Some(upstream) => value_tail(
                        ui,
                        &short_ref(upstream),
                        " · keeps its name there",
                    ),
                    None => value_tail(ui, "Not published", " · nothing changes there"),
                });
            }
        }),
    }
}

/// Delete branches: the local branches Git may delete, each with what Git would do.
pub(crate) fn delete_branches_body(
    ui: &mut Ui,
    dialog: &mut Dialog,
    tab: Option<&Tab>,
    backend: Option<&Backend>,
    widgets: &mut HashMap<String, UiControl>,
) {
    let all: Vec<&Branch> = crate::batch::deletable_branches(branches(tab)).collect();
    let names: Vec<Vec<u8>> = all.iter().map(|b| crate::batch::branch_name(b).to_vec()).collect();
    dialog.small.read_merge(backend, tab.map(|t| t.path.as_path()), &names);
    dialog.small.poll(ui.ctx());
    dialog.selected_branches.retain(|name| names.contains(name));
    let items = crate::batch::branch_items(branches(tab), &dialog.first);
    ui.horizontal(|ui| {
        let width = (ui.available_width() - 190.0).max(120.0);
        let r = search_field(ui, &mut dialog.first, "Filter branches", width);
        control(widgets, "dialog-branches-filter", &r, ui.clip_rect());
        let merged: Vec<Vec<u8>> = items
            .iter()
            .filter(|i| dialog.small.merge_of(&i.name).is_some_and(|s| s.merged))
            .map(|i| i.name.clone())
            .collect();
        let r = pages::ghost(ui, !merged.is_empty(), "Select merged");
        control(widgets, "dialog-branches-merged", &r, ui.clip_rect());
        if r.clicked() {
            for name in merged {
                if !dialog.selected_branches.contains(&name)
                    && dialog.selected_branches.len() < MAX_BRANCH_BATCH
                {
                    dialog.selected_branches.push(name);
                }
            }
        }
        let r = pages::ghost(ui, !dialog.selected_branches.is_empty(), "Clear");
        control(widgets, "dialog-branches-none", &r, ui.clip_rect());
        if r.clicked() {
            dialog.selected_branches.clear();
        }
    });
    if all.is_empty() {
        pages::empty(ui, "No other local branches to delete.");
    }
    let rows = ScrollArea::vertical()
        .id_salt("dialog-branches-list")
        .max_height(250.0)
        .auto_shrink([false, true])
        .show_rows(ui, 46.0, items.len(), |ui, range| {
            ui.spacing_mut().item_spacing.y = 2.0;
            for i in range {
                let item = &items[i];
                let branch = all.iter().find(|b| crate::batch::branch_name(b) == item.name);
                let mut on = dialog.selected_branches.contains(&item.name);
                let can = on || dialog.selected_branches.len() < MAX_BRANCH_BATCH;
                let pill = match dialog.small.merge_of(&item.name) {
                    Some(s) if s.merged => Some(("Merged".to_owned(), GREEN())),
                    Some(s) => Some((
                        match s.unmerged {
                            Some(n) => format!("{} not merged", plural(n as usize, "commit", "commits")),
                            None => "Not merged".into(),
                        },
                        AMBER(),
                    )),
                    None if dialog.small.merge_task.is_some() => Some(("Checking".into(), MUTED())),
                    None => None,
                };
                // An unmerged branch's tip is rarely in the loaded history; the merge
                // check read it.
                let commit = branch.and_then(|b| {
                    commit_in(tab, &b.target).or_else(|| {
                        dialog.small.merge_of(&item.name).map(|s| &s.tip).filter(|c| c.id == b.target)
                    })
                });
                let detail = match (branch, commit) {
                    (Some(b), Some(c)) => format!("{} · {}", &b.target.as_str()[..7], subject(c)),
                    (Some(b), None) => b.target.as_str()[..7].to_owned(),
                    _ => String::new(),
                };
                let when = commit.map_or_else(String::new, |c| age(c.author.timestamp));
                let r = branch_row(ui, &item.label, pill.as_ref().map(|(t, c)| (t.as_str(), *c)), &detail, &when, on, can);
                control(widgets, format!("dialog-branches-{i}"), &r, ui.clip_rect());
                if r.clicked() && can {
                    on = !on;
                    if on {
                        dialog.selected_branches.push(item.name.clone());
                    } else {
                        dialog.selected_branches.retain(|n| n != &item.name);
                    }
                }
            }
        });
    pages::region(
        widgets,
        "dialog-branches-scroll",
        rows.inner_rect.intersect(ui.clip_rect()),
        true,
    );
    dialog.selected_branches.sort();
    let held: Vec<String> = branches(tab)
        .iter()
        .filter(|b| b.kind == BranchKind::Local && b.symbolic_target.is_none())
        .filter(|b| b.is_head || b.checked_out_at.is_some())
        .map(|b| {
            let name = display(crate::batch::branch_name(b));
            match (&b.checked_out_at, b.is_head) {
                (_, true) => format!("{name} (current)"),
                (Some(path), false) => format!(
                    "{name} (checked out in {})",
                    path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned())
                ),
                (None, false) => name,
            }
        })
        .collect();
    if !held.is_empty() {
        pages::note(ui, &format!("Not listed: {}.", join_and(&held)));
    }
    if let Some(error) = &dialog.small.merge_error {
        pages::callout(ui, AMBER(), &format!("Couldn't check which branches are merged: {error}"));
    }
    if !dialog.selected_branches.is_empty() {
        let (color, text) = delete_outcome(dialog, tab);
        pages::callout(ui, color, &text);
    }
}

/// "a", "a and b", "a, b and c".
fn join_and(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// What deleting the selection does: blue when nothing is lost, amber when Git
/// keeps some.
fn delete_outcome(dialog: &Dialog, tab: Option<&Tab>) -> (Color32, String) {
    let selected = &dialog.selected_branches;
    let states: Vec<Option<&BranchMergeState>> =
        selected.iter().map(|n| dialog.small.merge_of(n)).collect();
    if states.iter().any(Option::is_none) {
        return (MUTED(), "Checking which of them are merged.".into());
    }
    let kept: Vec<&BranchMergeState> = states.iter().filter_map(|s| *s).filter(|s| !s.merged).collect();
    if kept.is_empty() {
        let into = if states.iter().flatten().any(|s| s.upstream) {
            String::new()
        } else {
            format!(" into {}", current(tab))
        };
        let subject = match selected.len() {
            1 => "It is merged".to_owned(),
            2 => "Both are merged".to_owned(),
            n => format!("All {n} are merged"),
        };
        return (BLUE(), format!("{subject}{into}, so no commits are lost. Branches on remotes stay."));
    }
    let deleted = selected.len() - kept.len();
    let first = match kept.as_slice() {
        [one] => format!(
            "{} has {} not merged into {}, so Git keeps it.",
            display(&one.name),
            match one.unmerged {
                Some(n) => plural(n as usize, "commit", "commits"),
                None => "commits".into(),
            },
            merge_base_name(one, tab)
        ),
        many => format!(
            "{} of them have commits that aren't merged, so Git keeps them.",
            many.len()
        ),
    };
    let rest = match deleted {
        0 => String::new(),
        1 => " The other one is deleted.".into(),
        n => format!(" The other {n} are deleted."),
    };
    (AMBER(), first + &rest)
}

/// What Git checks a branch against before deleting it.
fn merge_base_name(state: &BranchMergeState, tab: Option<&Tab>) -> String {
    if state.upstream { "its upstream".into() } else { current(tab) }
}

/// Delete branches' primary button and footer line.
pub(crate) fn delete_footer(dialog: &Dialog) -> (String, String) {
    let n = dialog.selected_branches.len();
    let kept = dialog
        .selected_branches
        .iter()
        .filter(|name| dialog.small.merge_of(name).is_some_and(|s| !s.merged))
        .count();
    let label = match n {
        0 => "Delete branches".to_owned(),
        n => format!("Delete {}", plural(n, "branch", "branches")),
    };
    let status = match (n, kept) {
        (0, _) => "Choose branches to delete".to_owned(),
        (n, 0) => format!("{n} selected"),
        (n, k) => format!("{n} selected · {k} will be kept"),
    };
    (label, status)
}

/// One branch in the delete list: a box, the name and what Git would do, then its
/// commit, and its age on the right. The whole row toggles it.
fn branch_row(
    ui: &mut Ui,
    name: &str,
    pill: Option<(&str, Color32)>,
    detail: &str,
    when: &str,
    on: bool,
    enabled: bool,
) -> Response {
    let (rect, response) = ui
        .add_enabled_ui(enabled, |ui| {
            ui.allocate_exact_size(vec2(ui.available_width(), 44.0), Sense::click())
        })
        .inner;
    let painter = ui.painter();
    if on {
        painter.rect_filled(rect, 8.0, CARD());
    } else if enabled && (response.hovered() || response.has_focus()) {
        painter.rect_filled(rect, 8.0, HOVER());
    }
    let ink = if enabled { TEXT() } else { MUTED() };
    let mark = Rect::from_center_size(pos2(rect.left() + 18.0, rect.center().y), vec2(15.0, 15.0));
    if on {
        painter.rect_filled(mark, 3.0, BLUE());
        icon(painter, mark.shrink(2.0), Icon::Check, ACCENT_INK());
    } else {
        painter.rect(mark, 3.0, BG(), Stroke::new(1.0, BORDER()), StrokeKind::Inside);
    }
    let right = rect.right() - 10.0;
    let age = painter.layout_no_wrap(when.to_owned(), FontId::proportional(12.0), HINT());
    let age_width = age.size().x;
    painter.galley(pos2(right - age_width, rect.center().y - age.size().y / 2.0), age, HINT());
    let left = rect.left() + 38.0;
    let width = (right - age_width - 12.0 - left).max(40.0);
    let pill = pill.map(|(text, color)| {
        (painter.layout_no_wrap(text.to_owned(), FontId::proportional(11.5), color), color)
    });
    let pill_width = pill.as_ref().map_or(0.0, |(g, _)| g.size().x + 14.0);
    let mut job = text::LayoutJob::simple_singleline(name.to_owned(), semibold(13.0), ink);
    job.wrap = text::TextWrapping::truncate_at_width((width - pill_width - 8.0).max(20.0));
    let title = painter.layout_job(job);
    let line = rect.top() + 14.0;
    let title_width = title.size().x;
    painter.galley(pos2(left, line - title.size().y / 2.0), title, ink);
    if let Some((galley, color)) = pill {
        let pill_rect = Rect::from_min_size(pos2(left + title_width + 8.0, line - 8.5), vec2(pill_width, 17.0));
        painter.rect_filled(pill_rect, 8.5, color.gamma_multiply(0.14));
        painter.galley(
            pos2(pill_rect.left() + 7.0, pill_rect.center().y - galley.size().y / 2.0),
            galley,
            color,
        );
    }
    let mut job = text::LayoutJob::simple_singleline(detail.to_owned(), FontId::proportional(12.0), HINT());
    job.wrap = text::TextWrapping::truncate_at_width(width);
    let detail = painter.layout_job(job);
    painter.galley(pos2(left, rect.bottom() - 13.0 - detail.size().y / 2.0), detail, HINT());
    response.widget_info(|| WidgetInfo::selected(WidgetType::Checkbox, enabled, on, name));
    response
}

/// Add remote can go ahead.
pub(crate) fn add_remote_ready(dialog: &Dialog, tab: Option<&Tab>) -> bool {
    !dialog.first.trim().is_empty()
        && !dialog.second.trim().is_empty()
        && remote_taken(&dialog.first, tab).is_none()
}

fn remote_taken(name: &str, tab: Option<&Tab>) -> Option<String> {
    let name = name.trim();
    snapshot(tab)?
        .remotes
        .iter()
        .find(|r| r.name == name.as_bytes())
        .map(|_| "A remote with this name already exists.".to_owned())
}

/// Add remote: its name, then the address with Test connection beside it.
pub(crate) fn add_remote_body(
    ui: &mut Ui,
    dialog: &mut Dialog,
    tab: Option<&Tab>,
    backend: Option<&Backend>,
    widgets: &mut HashMap<String, UiControl>,
) {
    label(ui, "Name");
    let r = line(ui, true, &mut dialog.first, "upstream");
    control(widgets, "dialog-remote-name", &r, ui.clip_rect());
    if let Some(problem) = remote_taken(&dialog.first, tab) {
        check(ui, false, &problem);
    }
    ui.add_space(2.0);
    label(ui, "URL or folder path");
    address_row(ui, dialog, tab, backend, widgets, "https://example.com/owner/repository.git");
    let name = dialog.first.trim();
    pages::note(
        ui,
        &format!(
            "Adding changes only this repository's settings. Fetch from {} afterwards to see its branches.",
            if name.is_empty() { "it" } else { name }
        ),
    );
}

/// The address field with Test connection at its end, and the test's result below.
fn address_row(
    ui: &mut Ui,
    dialog: &mut Dialog,
    tab: Option<&Tab>,
    backend: Option<&Backend>,
    widgets: &mut HashMap<String, UiControl>,
    hint: &str,
) {
    let target = crate::remote_check::target(dialog);
    let root = tab.map(|t| t.path.as_path());
    let add = matches!(dialog.kind, DialogKind::AddRemote);
    ui.horizontal(|ui| {
        let width = (ui.available_width() - 132.0).max(120.0);
        let text = if add { &mut dialog.second } else { &mut dialog.first };
        let r = ui.add_sized(
            vec2(width, 30.0),
            TextEdit::singleline(text)
                .hint_text(RichText::new(hint).color(FAINT()))
                .char_limit(4096)
                .vertical_align(Align::Center),
        );
        control(widgets, "dialog-remote-url", &r, ui.clip_rect());
        if r.changed() {
            dialog.error = None;
        }
        crate::remote_check::inline_button(ui, &mut dialog.connection, target, backend, root, widgets);
    });
    crate::remote_check::inline_status(ui, &dialog.connection);
}

/// Edit remote URL: which address, what it is now, and the new one.
pub(crate) fn edit_remote_body(
    ui: &mut Ui,
    dialog: &mut Dialog,
    tab: Option<&Tab>,
    backend: Option<&Backend>,
    widgets: &mut HashMap<String, UiControl>,
) {
    let DialogKind::EditRemote(remote, direction) = &dialog.kind else {
        return;
    };
    let (remote, direction) = (remote.clone(), *direction);
    let push = direction == RemoteUrlKind::Push;
    label(ui, "Which address");
    let items = [("Fetch URL", !push), ("Push URL", push)];
    let mut switch = None;
    ui.horizontal(|ui| {
        for (r, (kind, id)) in pages::segments(ui, &items)
            .iter()
            .zip([(RemoteUrlKind::Fetch, "fetch"), (RemoteUrlKind::Push, "push")])
        {
            control(widgets, format!("dialog-remote-which-{id}"), r, ui.clip_rect());
            if r.clicked() && kind != direction {
                switch = Some(kind);
            }
        }
    });
    if let Some(kind) = switch {
        let values = if kind == RemoteUrlKind::Fetch { &remote.urls } else { &remote.push_urls };
        dialog.first = values.first().map(|v| display(v)).unwrap_or_default();
        dialog.flag = kind == RemoteUrlKind::Push && values.is_empty();
        dialog.connection = None;
        dialog.error = None;
        dialog.kind = DialogKind::EditRemote(remote.clone(), kind);
        return;
    }
    let now = if push {
        remote.push_urls.first().map_or_else(|| "Same as the fetch URL".into(), |u| display(u))
    } else {
        remote.urls.first().map_or_else(|| "Not set".into(), |u| display(u))
    };
    pages::key_value(ui, "Now", KEY, |ui| {
        ui.add(Label::new(RichText::new(now).monospace().size(12.5).color(MUTED())).truncate());
    });
    if push {
        let r = ui.add(checkbox(&mut dialog.flag, "Push to the fetch URL"));
        control(widgets, "dialog-remote-fallback", &r, ui.clip_rect());
        if r.changed() {
            dialog.connection = None;
        }
    }
    if !(push && dialog.flag) {
        ui.add_space(2.0);
        label(ui, if push { "New push URL" } else { "New fetch URL" });
        address_row(ui, dialog, tab, backend, widgets, "https://example.com/owner/repository.git");
    } else {
        crate::remote_check::inline_status(ui, &dialog.connection);
    }
    pages::note(ui, "Saving changes only this repository's settings. Nothing is sent to the server.");
}

/// Edit remote URL: the form still says what the remote has, so saving changes nothing.
pub(crate) fn edit_remote_unchanged(dialog: &Dialog) -> bool {
    let DialogKind::EditRemote(remote, kind) = &dialog.kind else {
        return false;
    };
    let same = |urls: &[Vec<u8>]| urls.first().is_some_and(|u| display(u) == dialog.first.trim());
    match kind {
        RemoteUrlKind::Push if dialog.flag => remote.push_urls.is_empty(),
        RemoteUrlKind::Push => same(&remote.push_urls),
        RemoteUrlKind::Fetch => same(&remote.urls),
    }
}

/// A remote's name and fetch address, as the pickers show it.
fn remote_label(name: &[u8], tab: Option<&Tab>) -> String {
    let url = snapshot(tab)
        .and_then(|s| s.remotes.iter().find(|r| r.name == name))
        .and_then(|r| r.urls.first())
        .map(|u| display(u));
    match url {
        Some(url) => format!("{} · {url}", display(name)),
        None => display(name),
    }
}

/// A remote picker; the list stays above the dialog.
fn remote_picker(ui: &mut Ui, id: &str, dialog: &mut Dialog, tab: Option<&Tab>, width: f32) -> bool {
    let before = dialog.remote;
    let dialog_layer = ui.layer_id();
    ComboBox::from_id_salt(id)
        .width(width)
        .selected_text(
            dialog
                .remotes
                .get(dialog.remote)
                .map_or_else(|| "No remotes".into(), |r| remote_label(r, tab)),
        )
        .show_ui(ui, |ui| {
            // Modal and popup share a layer order; keep the list above the dialog.
            ui.ctx().set_sublayer(dialog_layer, ui.layer_id());
            for (i, remote) in dialog.remotes.iter().enumerate() {
                ui.selectable_value(&mut dialog.remote, i, remote_label(remote, tab));
            }
        });
    dialog.remote != before
}

/// Fetch: from which remote, and which tags.
pub(crate) fn fetch_body(
    ui: &mut Ui,
    dialog: &mut Dialog,
    tab: Option<&Tab>,
    widgets: &mut HashMap<String, UiControl>,
) {
    if dialog.remotes.is_empty() {
        no_remotes(ui, dialog, widgets, "Fetch needs a remote to download from.");
        return;
    }
    label(ui, "From");
    let width = ui.available_width();
    remote_picker(ui, "remote-picker", dialog, tab, width);
    ui.add_space(4.0);
    label(ui, "Tags");
    let choices = [
        ("none", FetchTags::None, "No tags", "Commits only"),
        ("default", FetchTags::RemoteDefault, "Usual tags", "Tags on fetched commits"),
        ("all", FetchTags::All, "All tags", "Never replaces yours"),
    ];
    let cards: Vec<_> = choices
        .iter()
        .map(|&(_, value, name, about)| (dialog.fetch_tags == value, name, about))
        .collect();
    for (r, &(id, value, ..)) in pages::option_cards(ui, true, &cards).iter().zip(&choices) {
        control(widgets, format!("dialog-fetch-tags-{id}"), r, ui.clip_rect());
        if r.clicked() {
            dialog.fetch_tags = value;
        }
    }
    let remote = dialog.remotes.get(dialog.remote).map(|r| display(r)).unwrap_or_default();
    pages::callout(
        ui,
        BLUE(),
        &format!(
            "Downloads new commits from {remote} and updates the {remote}/… branches. Your branches and files don't change."
        ),
    );
}

/// The empty state of Fetch and Push without remotes.
fn no_remotes(ui: &mut Ui, dialog: &mut Dialog, widgets: &mut HashMap<String, UiControl>, why: &str) {
    ui.add_space(6.0);
    ui.vertical_centered(|ui| {
        ui.label(RichText::new("No remotes yet").font(semibold(14.0)));
        ui.label(RichText::new(why).color(MUTED()));
        ui.add_space(4.0);
        let r = ui.button("Add remote");
        control(widgets, "dialog-add-remote", &r, ui.clip_rect());
        if r.clicked() {
            dialog.small.open = Some(DialogKind::AddRemote);
        }
    });
    ui.add_space(6.0);
}

/// Push: from the current branch to a branch on a remote, and what that sends.
pub(crate) fn push_body(
    ui: &mut Ui,
    dialog: &mut Dialog,
    tab: Option<&Tab>,
    backend: Option<&Backend>,
    widgets: &mut HashMap<String, UiControl>,
) {
    if dialog.remotes.is_empty() {
        no_remotes(ui, dialog, widgets, "Push needs a remote to send your commits to.");
        return;
    }
    let remote = dialog.remotes.get(dialog.remote).cloned().unwrap_or_default();
    let remote_name = display(&remote);
    // From → To, side by side.
    let gap = 28.0;
    let half = ((ui.available_width() - gap) / 2.0).floor();
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        ui.vertical(|ui| {
            ui.set_width(half);
            label(ui, "From");
            let (rect, _) = ui.allocate_exact_size(vec2(half, 30.0), Sense::hover());
            ui.painter().rect(rect, 6.0, PANEL(), Stroke::new(1.0, CARD_STROKE()), StrokeKind::Inside);
            icon(
                ui.painter(),
                Rect::from_center_size(pos2(rect.left() + 16.0, rect.center().y), vec2(13.0, 13.0)),
                Icon::Branch,
                HINT(),
            );
            clipped_text(
                ui,
                Rect::from_min_max(pos2(rect.left() + 30.0, rect.top()), pos2(rect.right() - 8.0, rect.bottom())),
                &display(&dialog.branch),
                13.0,
                TEXT(),
            );
        });
        let (rect, _) = ui.allocate_exact_size(vec2(gap, 48.0), Sense::hover());
        icon(
            ui.painter(),
            Rect::from_center_size(pos2(rect.center().x, rect.bottom() - 15.0), vec2(13.0, 13.0)),
            Icon::ChevronRight,
            HINT(),
        );
        ui.vertical(|ui| {
            ui.set_width(half);
            label(ui, &format!("To branch on {remote_name}"));
            let r = ui.add_sized(
                vec2(half, 30.0),
                TextEdit::singleline(&mut dialog.second)
                    .char_limit(4096)
                    .vertical_align(Align::Center),
            );
            control(widgets, "push-destination", &r, ui.clip_rect());
        });
    });
    // The remote and the destination's branches, then the upstream.
    let names: Vec<String> = snapshot(tab)
        .map(|s| {
            let prefix = [b"refs/remotes/".as_slice(), remote.as_slice(), b"/".as_slice()].concat();
            s.branches
                .iter()
                .filter(|b| b.symbolic_target.is_none())
                .filter_map(|b| b.reference.strip_prefix(prefix.as_slice()))
                .map(display)
                .collect()
        })
        .unwrap_or_default();
    let mut upstream_note = None;
    ui.horizontal(|ui| {
        if dialog.remotes.len() > 1 {
            label(ui, "Remote");
            if remote_picker(ui, "remote-picker", dialog, tab, 140.0) {
                dialog.small.outlook = None;
            }
        }
        if let Some(name) = crate::branch_picker::pick(
            ui,
            widgets,
            "push-destination-choice",
            &format!("Branches on {remote_name}"),
            &names,
            !names.is_empty(),
        ) {
            dialog.second = name;
        }
        let tracking = [b"refs/remotes/".as_slice(), remote.as_slice(), b"/", dialog.second.trim().as_bytes()].concat();
        let tracks = branches(tab).iter().any(|b| {
            b.kind == BranchKind::Local
                && crate::batch::branch_name(b) == dialog.branch.as_slice()
                && b.upstream.as_deref() == Some(tracking.as_slice())
        });
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if tracks {
                dialog.flag = false;
                upstream_note = Some(format!("Already tracks {remote_name}/{}", dialog.second.trim()));
            } else {
                let r = ui.add(checkbox(&mut dialog.flag, "Track it here (set upstream)"));
                control(widgets, "push-upstream", &r, ui.clip_rect());
            }
            if let Some(note) = &upstream_note {
                ui.label(RichText::new(note).size(12.0).color(HINT()));
            }
        });
    });
    // What the push sends, as of the last fetch.
    let target: Target = (remote.clone(), dialog.branch.clone(), dialog.second.trim().to_owned());
    dialog.small.read_outlook(target.clone(), backend, tab.map(|t| t.path.as_path()));
    dialog.small.poll(ui.ctx());
    let destination = dialog.second.trim().to_owned();
    let mut lease = false;
    let mut pull = false;
    card(ui, "What will be pushed", |ui| match dialog.small.outlook_for(&target) {
        None if destination.is_empty() => pages::note(ui, "Enter the branch to push to."),
        None => pages::busy_line(ui, &format!("Checking what {remote_name} has")),
        Some(Err(error)) => pages::callout(ui, AMBER(), &format!("Couldn't compare with {remote_name}: {error}")),
        Some(Ok(outlook)) => {
            for c in outlook.commits.iter().take(LISTED) {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    ui.label(RichText::new(&c.id.as_str()[..7]).monospace().size(12.5).color(MUTED()));
                    let when = format!("{} · {}", display(&c.author.name), age(c.author.timestamp));
                    let width = ui.painter().layout_no_wrap(when.clone(), FontId::proportional(12.0), HINT()).size().x;
                    pages::left_label(ui, (ui.available_width() - width - 8.0).max(40.0), Label::new(subject(c)));
                    ui.label(RichText::new(when).size(12.0).color(HINT()));
                });
            }
            let more = outlook.ahead as usize - outlook.commits.len().min(LISTED);
            if more > 0 {
                pages::note(ui, &format!("and {} more", plural(more, "commit", "commits")));
            }
            let (color, text) = push_outcome(outlook, &remote_name, &destination);
            pages::callout(ui, color, &text);
            if outlook.behind > 0 {
                ui.horizontal(|ui| {
                    let r = ui.button("Pull first");
                    control(widgets, "push-pull-first", &r, ui.clip_rect());
                    pull = r.clicked();
                    let r = pages::ghost(ui, true, "Review force push with lease");
                    control(widgets, "push-lease-open", &r, ui.clip_rect());
                    lease = r.clicked();
                });
            }
        }
    });
    let behind = dialog
        .small
        .outlook_for(&target)
        .is_some_and(|r| r.as_ref().is_ok_and(|o| o.behind > 0));
    if !behind {
        let r = pages::ghost(
            ui,
            !dialog.branch.is_empty() && !destination.is_empty(),
            "Force push with lease",
        );
        control(widgets, "push-lease-open", &r, ui.clip_rect());
        lease |= r.clicked();
    }
    if lease {
        dialog.force_push = Some(Default::default());
        dialog.error = None;
    }
    if pull {
        dialog.small.open = Some(DialogKind::Pull);
    }
}

/// What the push will do, from the outlook.
fn push_outcome(outlook: &PushOutlook, remote: &str, destination: &str) -> (Color32, String) {
    let commits = plural(outlook.ahead as usize, "commit", "commits");
    if outlook.behind > 0 {
        return (
            AMBER(),
            format!(
                "As of your last fetch, {remote} has {} you don't have. This push will be refused until you pull them.",
                plural(outlook.behind as usize, "commit", "commits")
            ),
        );
    }
    match (&outlook.tracked, outlook.ahead) {
        (None, _) => (BLUE(), format!("Creates {destination} on {remote} with {commits}.")),
        (Some(_), 0) => (BLUE(), format!("{remote}/{destination} already has everything. Nothing to push.")),
        (Some(_), _) => (
            BLUE(),
            format!("{remote}/{destination} moves forward {commits}. Nothing else changes on {remote}."),
        ),
    }
}

/// Push's footer line.
fn push_status(dialog: &Dialog) -> String {
    let remote = dialog.remotes.get(dialog.remote).map(|r| display(r)).unwrap_or_default();
    let target: Target = (
        dialog.remotes.get(dialog.remote).cloned().unwrap_or_default(),
        dialog.branch.clone(),
        dialog.second.trim().to_owned(),
    );
    match dialog.small.outlook_for(&target) {
        Some(Ok(o)) if o.behind > 0 => "Likely to be refused · pull first".into(),
        Some(Ok(o)) if o.ahead == 0 && o.tracked.is_some() => "Nothing to push".into(),
        Some(Ok(o)) => format!("{} to {remote}", plural(o.ahead as usize, "commit", "commits")),
        _ if dialog.second.trim().is_empty() => "Enter the branch to push to".into(),
        _ => format!("To {remote}"),
    }
}

/// Changed files a full stash saves, and how many are new (untracked).
fn stash_counts(entries: &[StatusEntry]) -> (usize, usize) {
    let new = entries.iter().filter(|e| e.kind == EntryKind::Untracked).count();
    let tracked = entries
        .iter()
        .filter(|e| !matches!(e.kind, EntryKind::Untracked | EntryKind::Ignored))
        .count();
    (tracked, new)
}

/// The Git status letter Changes shows for an entry.
fn status_letter(entry: &StatusEntry) -> char {
    match entry.kind {
        EntryKind::Untracked => 'U',
        EntryKind::Unmerged => 'C',
        _ => {
            let change = if matches!(entry.worktree, Change::Unchanged) { entry.index } else { entry.worktree };
            match change {
                Change::Added => 'A',
                Change::Deleted => 'D',
                Change::Renamed | Change::Copied => 'R',
                _ => 'M',
            }
        }
    }
}

/// Save changes to stash: a message, what to save, and the files that go.
pub(crate) fn stash_body(
    ui: &mut Ui,
    dialog: &mut Dialog,
    tab: Option<&Tab>,
    widgets: &mut HashMap<String, UiControl>,
) {
    label(ui, "Message · optional");
    let r = line(ui, true, &mut dialog.first, "What you were working on");
    control(widgets, "dialog-stash-message", &r, ui.clip_rect());
    let Some(status) = snapshot(tab).and_then(|s| s.status.as_ref()) else {
        pages::waiting(ui, "Reading changes");
        return;
    };
    let (tracked, new) = stash_counts(&status.entries);
    if tracked + new == 0 {
        pages::callout(ui, MUTED(), "Nothing to save: the working folder has no changes.");
        return;
    }
    ui.horizontal(|ui| {
        let selected = dialog.stash_paths.is_some();
        let items = [("All changes", !selected), ("Selected files", selected)];
        for (r, (only, id)) in pages::segments(ui, &items)
            .iter()
            .zip([(false, "dialog-stash-all-changes"), (true, "dialog-stash-selected")])
        {
            control(widgets, id, r, ui.clip_rect());
            if r.clicked() && only != selected {
                dialog.stash_paths = only.then(Vec::new);
            }
        }
        ui.add_space(8.0);
        let r = ui.add_enabled(new > 0, checkbox(&mut dialog.flag, format!("Include new files ({new})")));
        control(widgets, "dialog-stash-untracked", &r, ui.clip_rect());
    });
    if let Some(selected) = dialog.stash_paths.as_mut() {
        crate::stash_save::checklist(ui, &status.entries, dialog.flag, selected, widgets);
        return;
    }
    let saved = tracked + if dialog.flag { new } else { 0 };
    card(ui, &format!("What will be saved · {}", plural(saved, "file", "files")), |ui| {
        let files: Vec<&StatusEntry> = status
            .entries
            .iter()
            .filter(|e| e.kind != EntryKind::Ignored)
            .collect();
        let shown = files.len().min(8);
        for entry in &files[..shown] {
            let stays = entry.kind == EntryKind::Untracked && !dialog.flag;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                let (badge, _) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
                let letter = status_letter(entry);
                status_badge(ui.painter(), badge.center(), letter);
                let path = entry.path.display_lossy();
                let (dir, file) = match path.rsplit_once('/') {
                    Some((dir, file)) => (dir.to_owned(), file.to_owned()),
                    None => (String::new(), path.into_owned()),
                };
                let note = if stays {
                    "stays in the folder"
                } else if entry.kind == EntryKind::Untracked {
                    "new file"
                } else if entry.staged() {
                    "staged"
                } else {
                    ""
                };
                let ink = if stays { MUTED() } else { TEXT() };
                let mut job = text::LayoutJob::default();
                job.append(&file, 0.0, TextFormat::simple(FontId::proportional(13.0), ink));
                if !dir.is_empty() {
                    job.append(&format!("  {dir}"), 0.0, TextFormat::simple(FontId::proportional(12.0), FAINT()));
                }
                let note_width = ui.painter().layout_no_wrap(note.to_owned(), FontId::proportional(12.0), HINT()).size().x;
                job.wrap = text::TextWrapping::truncate_at_width((ui.available_width() - note_width - 8.0).max(40.0));
                ui.label(job);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(RichText::new(note).size(12.0).color(HINT()));
                });
            });
        }
        if files.len() > shown {
            pages::note(ui, &format!("and {} more", plural(files.len() - shown, "file", "files")));
        }
    });
    pages::note(ui, "Your working folder becomes clean. Apply the stash later from Stashes.");
}

/// Save stash's footer line.
fn stash_status(dialog: &Dialog, tab: Option<&Tab>) -> String {
    let Some(status) = snapshot(tab).and_then(|s| s.status.as_ref()) else {
        return String::new();
    };
    let (tracked, new) = stash_counts(&status.entries);
    if tracked + new == 0 {
        return "Nothing to save".into();
    }
    match &dialog.stash_paths {
        Some(paths) => format!("{} selected · staged files always go", plural(paths.len(), "file", "files")),
        None => format!(
            "{} · working folder becomes clean",
            plural(tracked + if dialog.flag { new } else { 0 }, "file", "files")
        ),
    }
}

/// Save stash can go ahead: there is something to save, and a partial stash has
/// at least one file.
pub(crate) fn stash_ready(dialog: &Dialog, tab: Option<&Tab>) -> bool {
    let has = snapshot(tab)
        .and_then(|s| s.status.as_ref())
        .is_some_and(|s| {
            let (tracked, new) = stash_counts(&s.entries);
            tracked + new > 0
        });
    has && dialog.stash_paths.as_ref().is_none_or(|p| !p.is_empty())
}

/// Why an identity value would be refused, in words.
fn identity_problem(value: &str, email: bool) -> Option<&'static str> {
    if value.is_empty() {
        return None;
    }
    if value.trim() != value {
        return Some("Remove spaces at the start or end.");
    }
    if value.chars().any(|c| c.is_control() || "<>".contains(c)) {
        return Some("Leave out < and > and control characters.");
    }
    if email
        && (value.chars().any(char::is_whitespace)
            || !value
                .split_once('@')
                .is_some_and(|(a, b)| !a.is_empty() && !b.is_empty() && !b.contains('@')))
    {
        return Some("Enter an email such as name@example.com.");
    }
    None
}

/// Repository settings, Identity: name and email, what applies without them, and
/// how commits made here will show.
pub(crate) fn identity_body(
    ui: &mut Ui,
    dialog: &mut Dialog,
    widgets: &mut HashMap<String, UiControl>,
) {
    let Some(identity) = dialog.identity.clone() else {
        return;
    };
    for (email, key) in [(false, "dialog-identity-name"), (true, "dialog-identity-email")] {
        label(ui, if email { "Email" } else { "Name" });
        let inherited = if email { &identity.inherited.email } else { &identity.inherited.name };
        let text = if email { &mut dialog.second } else { &mut dialog.first };
        let r = line(ui, true, text, inherited.as_deref().unwrap_or(""));
        control(widgets, key, &r, ui.clip_rect());
        if r.changed() {
            dialog.error = None;
        }
        let text = if email { &dialog.second } else { &dialog.first };
        match identity_problem(text, email) {
            Some(problem) => check(ui, false, problem),
            None => pages::note(
                ui,
                &format!(
                    "Everywhere else: {} · leave empty to use that",
                    inherited.as_deref().unwrap_or("not set")
                ),
            ),
        }
        ui.add_space(2.0);
    }
    let name = Some(dialog.first.trim()).filter(|n| !n.is_empty()).or(identity.inherited.name.as_deref());
    let email = Some(dialog.second.trim()).filter(|e| !e.is_empty()).or(identity.inherited.email.as_deref());
    card(ui, "Your commits here will show", |ui| match (name, email) {
        (Some(name), Some(email)) => {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                let (avatar, _) = ui.allocate_exact_size(vec2(28.0, 28.0), Sense::hover());
                ui.painter().circle_filled(avatar.center(), 14.0, SELECTED());
                let initials: String = name
                    .split_whitespace()
                    .filter_map(|w| w.chars().next())
                    .take(2)
                    .flat_map(char::to_uppercase)
                    .collect();
                ui.painter().text(avatar.center(), Align2::CENTER_CENTER, initials, semibold(11.0), BLUE());
                let mut job = text::LayoutJob::default();
                job.append(name, 0.0, TextFormat::simple(semibold(13.0), TEXT()));
                job.append(&format!(" <{email}>"), 0.0, TextFormat::simple(FontId::proportional(13.0), MUTED()));
                job.wrap = text::TextWrapping::truncate_at_width(ui.available_width());
                ui.label(job);
            });
        }
        _ => pages::callout(ui, AMBER(), "Git needs a name and an email before you can commit here."),
    });
    if (identity.local.name.is_some() && identity.local.name != identity.effective.name)
        || (identity.local.email.is_some() && identity.local.email != identity.effective.email)
    {
        pages::callout(
            ui,
            AMBER(),
            "Another configuration overrides a local value. Review worktree/include settings with Git.",
        );
    }
}

/// Repository settings' identity can be saved.
pub(crate) fn identity_ready(dialog: &Dialog) -> bool {
    identity_problem(&dialog.first, false).is_none() && identity_problem(&dialog.second, true).is_none()
}

/// The body of a confirmation that has more to show than its title: what it acts on,
/// in one card, and what happens.
pub(crate) fn confirm_body(
    ui: &mut Ui,
    dialog: &mut Dialog,
    tab: Option<&Tab>,
    backend: Option<&Backend>,
    widgets: &mut HashMap<String, UiControl>,
) {
    let DialogKind::Confirm(action, _) = &dialog.kind else {
        return;
    };
    match action.clone() {
        Action::DeleteBranch(name) => {
            dialog.small.read_merge(backend, tab.map(|t| t.path.as_path()), std::slice::from_ref(&name));
            dialog.small.poll(ui.ctx());
            let mut full = b"refs/heads/".to_vec();
            full.extend_from_slice(&name);
            let branch = branches(tab).iter().find(|b| b.reference == full);
            let state = dialog.small.merge_of(&name);
            card(ui, "Branch", |ui| {
                if let Some(b) = branch {
                    let tip = commit_in(tab, &b.target)
                        .or_else(|| state.map(|s| &s.tip).filter(|c| c.id == b.target));
                    pages::key_value(ui, "Points to", KEY, |ui| commit_ref(ui, &b.target, tip, ""));
                }
                pages::key_value(ui, "Merged", KEY, |ui| match state {
                    Some(s) if s.merged => value_tail(
                        ui,
                        "Yes",
                        &if s.upstream { " · into its upstream".into() } else { format!(" · into {}", current(tab)) },
                    ),
                    Some(s) => value_tail(
                        ui,
                        "No",
                        &match s.unmerged {
                            Some(n) => format!(
                                " · {} not in {}",
                                plural(n as usize, "commit", "commits"),
                                merge_base_name(s, tab)
                            ),
                            None => String::new(),
                        },
                    ),
                    None => value_tail(ui, "Checking", ""),
                });
                if let Some(b) = branch {
                    pages::key_value(ui, "On a remote", KEY, |ui| match &b.upstream {
                        Some(upstream) => value_tail(ui, &short_ref(upstream), " · stays there"),
                        None => value(ui, "Not published"),
                    });
                }
            });
            let (color, text) = match state {
                Some(s) if s.merged => {
                    (BLUE(), format!("No commits are lost: they are all in {}.", merge_base_name(s, tab)))
                }
                Some(s) => (
                    AMBER(),
                    format!("It has commits that aren't in {}, so Git keeps it.", merge_base_name(s, tab)),
                ),
                None => (MUTED(), "Checking whether it is merged.".to_owned()),
            };
            pages::callout(ui, color, &text);
        }
        Action::RemoveRemote(remote) => {
            let prefix = [b"refs/remotes/".as_slice(), remote.name.as_slice(), b"/"].concat();
            let tracking: Vec<String> = branches(tab)
                .iter()
                .filter(|b| b.symbolic_target.is_none() && b.reference.starts_with(&prefix))
                .map(|b| short_ref(&b.reference))
                .collect();
            let tracked_by: Vec<String> = branches(tab)
                .iter()
                .filter(|b| b.kind == BranchKind::Local)
                .filter(|b| b.upstream.as_ref().is_some_and(|u| u.starts_with(&prefix)))
                .map(|b| display(crate::batch::branch_name(b)))
                .collect();
            card(ui, "What goes with it", |ui| {
                pages::key_value(ui, "Address", KEY, |ui| {
                    value(ui, &remote.urls.first().map_or_else(|| "Not set".into(), |u| display(u)));
                });
                pages::key_value(ui, "Remote branches", KEY, |ui| {
                    let names = tracking.iter().take(3).cloned().collect::<Vec<_>>().join(", ");
                    let more = tracking.len().saturating_sub(3);
                    value_tail(
                        ui,
                        &tracking.len().to_string(),
                        &if names.is_empty() {
                            String::new()
                        } else if more > 0 {
                            format!(" · {names} and {more} more")
                        } else {
                            format!(" · {names}")
                        },
                    );
                });
                pages::key_value(ui, "Tracked by", KEY, |ui| {
                    if tracked_by.is_empty() {
                        value(ui, "No local branches");
                    } else {
                        value_tail(ui, &join_and(&tracked_by), " · they stop tracking it");
                    }
                });
            });
            pages::callout(ui, BLUE(), "Only this repository's settings change. Nothing is deleted on the server.");
        }
        Action::DropStash(entry) | Action::ApplyStash { entry, .. } => {
            let drop = matches!(action, Action::DropStash(_));
            let files = stash_files(tab, &entry);
            let title = match &files {
                Some(files) => format!("What it holds · {}", plural(files.len(), "file", "files")),
                None => "What it holds".into(),
            };
            card(ui, &title, |ui| {
                pages::key_value(ui, "Message", KEY, |ui| value(ui, &display(&entry.subject)));
                if let Some(files) = &files {
                    for (i, (name, added, removed)) in files.iter().take(6).enumerate() {
                        pages::key_value(ui, if i == 0 { "Files" } else { "" }, KEY, |ui| {
                            value_tail(ui, name, &format!(" · +{added} −{removed}"));
                        });
                    }
                    if files.len() > 6 {
                        pages::key_value(ui, "", KEY, |ui| value_tail(ui, "", &format!("and {} more", files.len() - 6)));
                    }
                }
                pages::key_value(ui, "Stash commit", KEY, |ui| {
                    ui.label(RichText::new(&entry.id.as_str()[..7]).monospace().size(12.5));
                });
            });
            if drop {
                pages::callout(
                    ui,
                    AMBER(),
                    "It leaves the Stashes list. Until Git cleans up, you can still find it under Reflog & recovery › Lost commits.",
                );
            } else {
                let r = ui.add(checkbox(&mut dialog.flag, "Restore the saved index (staging split)"));
                control(widgets, "dialog-stash-index", &r, ui.clip_rect());
            }
        }
        _ => {}
    }
}

/// The selected stash's files on the Stashes page, when that stash is shown there.
fn stash_files(tab: Option<&Tab>, entry: &StashEntry) -> Option<Vec<(String, usize, usize)>> {
    let tab = tab?;
    if tab.stash.selected.as_ref().map(|s| &s.id) != Some(&entry.id) {
        return None;
    }
    let patch = tab.stash.patch.as_ref()?;
    Some(patch.files.iter().map(|f| (f.label.clone(), f.added, f.removed)).collect())
}

/// Close repository: what is kept for next time.
pub(crate) fn close_body(ui: &mut Ui, tab: Option<&Tab>) {
    let Some(tab) = tab else {
        return;
    };
    let draft = tab.draft.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim();
    let changed = snapshot(Some(tab))
        .and_then(|s| s.status.as_ref())
        .map_or(0, |s| s.entries.iter().filter(|e| e.kind != EntryKind::Ignored).count());
    card(ui, "Kept for next time", |ui| {
        if !draft.is_empty() {
            pages::key_value(ui, "Message draft", KEY, |ui| value(ui, draft));
        }
        pages::key_value(ui, "Changed files", KEY, |ui| {
            if changed == 0 {
                value(ui, "None");
            } else {
                value_tail(ui, &changed.to_string(), " · they stay in the folder");
            }
        });
    });
    if tab.conflict.dirty {
        pages::callout(ui, AMBER(), "Unsaved conflict-editor text will be discarded.");
    }
    pages::callout(ui, BLUE(), "Reopen it from File › Open recent.");
}

/// Exit: what is running and what is kept, one row each.
pub(crate) fn quit_body(ui: &mut Ui, rows: &[(Color32, Icon, String)]) {
    for (color, image, text) in rows {
        Frame::new()
            .fill(color.gamma_multiply(0.08))
            .corner_radius(8)
            .inner_margin(Margin::symmetric(12, 9))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = 10.0;
                    let (mark, _) = ui.allocate_exact_size(vec2(15.0, 17.0), Sense::hover());
                    icon(ui.painter(), mark.shrink2(vec2(0.0, 1.0)), *image, *color);
                    ui.add(Label::new(RichText::new(text).color(if *color == BLUE() { TEXT() } else { *color })).wrap());
                });
            });
    }
}

impl Rocket {
    /// Exit: what is running or unsaved, then what is kept, one row each.
    pub(crate) fn quit_rows(&self) -> Vec<(Color32, Icon, String)> {
        let mut rows = Vec::new();
        if let Some(write) = &self.write {
            rows.push((AMBER(), Icon::Stop, format!("{} is still running. Exiting stops it first.", write.title)));
        }
        if let Some(batch) = &self.manager.status.batch {
            rows.push((AMBER(), Icon::Stop, format!("{} is still running. Exiting stops it first.", batch.kind.title())));
        }
        if self.external.running() {
            rows.push((AMBER(), Icon::External, "The running external tool will be stopped.".into()));
        }
        if self.tabs.iter().any(|t| t.conflict.dirty) {
            rows.push((AMBER(), Icon::Warning, "Unsaved conflict-editor text will be discarded.".into()));
        }
        if self.forge.pull.has_draft() {
            rows.push((AMBER(), Icon::Warning, "Unsent pull request review text and inline comments will be saved locally before closing. They are not submitted; save failures keep this window open.".into()));
        }
        match &self.state_error {
            Some(error) => rows.push((
                RED(),
                Icon::Warning,
                format!("{error} Exiting now loses unsaved drafts."),
            )),
            None => {
                let drafts = self.tabs.iter().filter(|t| !t.draft.trim().is_empty()).count();
                rows.push((
                    BLUE(),
                    Icon::Check,
                    match drafts {
                        0 => "Your open repositories are remembered for next time.".into(),
                        1 => "The message draft in 1 repository and your open repositories are saved.".into(),
                        n => format!("Message drafts in {n} repositories and your open repositories are saved."),
                    },
                ));
            }
        }
        rows
    }
}

/// The footer line of the small dialogs; empty for the rest.
pub(crate) fn status(dialog: &Dialog, tab: Option<&Tab>) -> String {
    match &dialog.kind {
        DialogKind::NewBranch => {
            let name = dialog.first.trim();
            if name.is_empty() {
                "Name the branch".into()
            } else if branch_name_ready(dialog, tab) {
                let at = snapshot(tab)
                    .and_then(|s| s.head.as_ref())
                    .and_then(|h| match h {
                        Head::Branch { target, .. } | Head::Detached { target } => Some(target.as_str()[..7].to_owned()),
                        Head::Unborn { .. } => None,
                    })
                    .map(|sha| format!(" at {sha}"))
                    .unwrap_or_default();
                format!("Creates {name}{at}")
            } else {
                "Choose another name".into()
            }
        }
        DialogKind::TrackBranch { upstream, .. } if !branches(tab).iter().any(|b| b.reference == *upstream) => {
            // As the card says; the name may be fine.
            "The remote branch is gone".into()
        }
        DialogKind::TrackBranch { .. } => {
            let name = dialog.first.trim();
            if name.is_empty() {
                "Name the branch".into()
            } else if branch_name_ready(dialog, tab) {
                // The card above names the upstream; with it, a long name cut this line.
                format!("Creates {name} and switches to it")
            } else {
                "Choose another name".into()
            }
        }
        DialogKind::Rename(_) => {
            if branch_name_ready(dialog, tab) {
                "Renames the local branch only".into()
            } else {
                "Choose another name".into()
            }
        }
        DialogKind::DeleteBranches => delete_footer(dialog).1,
        DialogKind::AddRemote => {
            let name = dialog.first.trim();
            if name.is_empty() {
                "Name the remote".into()
            } else if dialog.connection.as_ref().is_some_and(|t| matches!(t.result, Some(Err(_)))) {
                "You can still add it; fetching fails until the address works".into()
            } else {
                format!("Adds {name}")
            }
        }
        DialogKind::EditRemote(..) if edit_remote_unchanged(dialog) => "No changes yet".into(),
        DialogKind::EditRemote(remote, kind) => {
            if dialog.connection.as_ref().is_some_and(|t| matches!(t.result, Some(Err(_)))) {
                "You can still save it; fetching fails until the address works".into()
            } else {
                format!(
                    "Changes {}'s {} URL",
                    display(&remote.name),
                    if *kind == RemoteUrlKind::Push { "push" } else { "fetch" }
                )
            }
        }
        DialogKind::Fetch if dialog.remotes.is_empty() => "Add a remote first".into(),
        DialogKind::Fetch => format!(
            "From {}",
            dialog.remotes.get(dialog.remote).map(|r| display(r)).unwrap_or_default()
        ),
        DialogKind::Push if dialog.remotes.is_empty() => "Add a remote first".into(),
        DialogKind::Push => push_status(dialog),
        DialogKind::Stash => stash_status(dialog, tab),
        DialogKind::RepositorySettings => "Saved to .git/config".into(),
        DialogKind::Confirm(Action::DropStash(_), _) => "Can't be undone from Stashes".into(),
        DialogKind::Confirm(Action::DeleteBranch(_), _) => "Deletes 1 local branch".into(),
        DialogKind::Confirm(Action::RemoveRemote(remote), _) => format!("Removes {}", display(&remote.name)),
        DialogKind::CloseTab(_) => "Nothing is lost".into(),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsent_reviews_warn_on_close_and_cancel_keeps_text() {
        use crate::{batch::flow_tests::{click, open}, test_common as common};
        common::isolated(
            "ROCKET_SMALL_DIALOGS_UI_CASE",
            "small_dialogs::tests::unsent_reviews_warn_on_close_and_cancel_keeps_text",
            || {
                let f = common::Fixture::new();
                f.write("a.txt", "base\n");
                f.commit("base");
                let ctx = Context::default();
                let mut app = open(&ctx, &f);
                for inline in [false, true] {
                    app.forge.pull = Default::default();
                    if inline {
                        app.forge.pull.inline_body = "PRIVATE unfinished inline review".into();
                    } else {
                        app.forge.pull.body = "PRIVATE unsent review".into();
                    }
                    let body = app.forge.pull.body.clone();
                    let inline_body = app.forge.pull.inline_body.clone();
                    app.request_close();
                    assert!(!app.quitting, "native close must first warn about unsent reviews");
                    assert!(app.dialog.as_ref().is_some_and(|d| matches!(d.kind, DialogKind::Quit)));
                    assert!(app.quit_rows().iter().any(|(_, _, text)|
                        text.contains("Unsent pull request review") && text.contains("saved locally")));
                    click(&ctx, &mut app, "dialog-cancel");
                    assert!(app.dialog.is_none());
                    assert!(!app.quitting);
                    assert_eq!(app.forge.pull.body, body);
                    assert_eq!(app.forge.pull.inline_body, inline_body);
                }
                // The menu Exit path uses the same warning and still allows an
                // explicit discard, without submitting anything to a provider.
                app.dispatch(Intent::Exit);
                assert!(app.quit_rows().iter().any(|(_, _, text)| text.contains("Unsent pull request review")));
                click(&ctx, &mut app, "dialog-submit");
                assert!(app.quitting);
            },
        );
    }

    fn local(name: &str) -> Branch {
        Branch {
            reference: format!("refs/heads/{name}").into_bytes(),
            target: ObjectId::parse(&"1".repeat(40)).unwrap(),
            kind: BranchKind::Local,
            is_head: false,
            upstream: None,
            symbolic_target: None,
            checked_out_at: None,
            committed: None,
            ahead_behind: None,
        }
    }

    #[test]
    fn branch_names_are_checked_as_you_type() {
        let branches = [local("main"), local("feature/login")];
        let problem = |name: &str| branch_name_problem(name, &branches, None);
        assert_eq!(problem("feature/search"), None);
        assert!(problem("main").unwrap().contains("already exists"));
        assert!(problem("cache keys").unwrap().ends_with("Try cache-keys."));
        assert!(problem("feature").unwrap().contains("both a branch and a folder"));
        assert!(problem("feature/login/more").unwrap().contains("both a branch and a folder"));
        assert!(problem("MAIN").unwrap().contains("letter case"));
        for bad in ["-x", "a..b", "a~1", "x.lock", "a/", "@", "refs/heads/x", "a:b"] {
            assert!(problem(bad).is_some(), "{bad}");
        }
        // A rename to the same name says so; the branch's own name isn't taken.
        assert_eq!(
            branch_name_problem("main", &branches, Some(b"main")).as_deref(),
            Some("That is its current name.")
        );
        assert_eq!(branch_name_problem("main-2", &branches, Some(b"main")), None);
    }

    /// Edit remote URL saves only a change; pushing to the fetch URL is no change
    /// for a remote without its own push URL.
    #[test]
    fn edit_remote_saves_only_a_change() {
        use crate::{
            batch::flow_tests::{click, enabled, open, wait},
            test_common as common,
        };
        common::isolated(
            "ROCKET_SMALL_DIALOGS_UI_CASE",
            "small_dialogs::tests::edit_remote_saves_only_a_change",
            || {
                let f = common::Fixture::new();
                f.write("a.txt", "1\n");
                f.commit("base");
                f.run(&["remote", "add", "origin", "https://example.invalid/a.git"]);
                let ctx = Context::default();
                let mut app = open(&ctx, &f);
                let remote = app.tab().unwrap().snapshot.as_ref().unwrap().remotes[0].clone();
                app.dispatch(Intent::Dialog(DialogKind::EditRemote(remote, RemoteUrlKind::Fetch)));
                wait(&ctx, &mut app);
                assert!(!enabled(&ctx, &mut app, "dialog-submit"), "it has this URL already");
                app.dialog.as_mut().unwrap().first = "https://example.invalid/b.git".into();
                assert!(enabled(&ctx, &mut app, "dialog-submit"));
                click(&ctx, &mut app, "dialog-remote-which-push");
                assert!(!enabled(&ctx, &mut app, "dialog-submit"), "it pushes to the fetch URL already");
            },
        );
    }

    /// A remote branch no local branch tracks opens New branch, starting from and
    /// tracking it. A taken name is refused; Create and switch checks it out.
    #[test]
    fn a_remote_branch_reviews_a_new_tracking_branch_and_switches_to_it() {
        use crate::{
            batch::flow_tests::{click, enabled, open, wait},
            branch_switcher::{Pick, listing},
            test_common as common,
        };
        common::isolated(
            "ROCKET_SMALL_DIALOGS_UI_CASE",
            "small_dialogs::tests::a_remote_branch_reviews_a_new_tracking_branch_and_switches_to_it",
            || {
                let f = common::Fixture::new();
                f.write("a.txt", "1\n");
                f.commit("base");
                let home = String::from_utf8(f.run(&["branch", "--show-current"])).unwrap().trim().to_owned();
                f.run(&["switch", "-c", "topic"]);
                f.write("a.txt", "2\n");
                f.commit("topic work");
                let topic = String::from_utf8(f.run(&["rev-parse", "HEAD"])).unwrap().trim().to_owned();
                f.run(&["switch", home.as_str()]);
                // As a fetch leaves it; nothing contacts the remote.
                f.run(&["remote", "add", "origin", "https://example.invalid/rocket.git"]);
                f.run(&["update-ref", "refs/remotes/origin/topic", topic.as_str()]);
                f.run(&["branch", "-D", "topic"]);
                let ctx = Context::default();
                let mut app = open(&ctx, &f);
                let snapshot = app.tab().unwrap().snapshot.as_ref().unwrap();
                let remotes: Vec<Vec<u8>> = snapshot.remotes.iter().map(|r| r.name.clone()).collect();
                let list = listing(&snapshot.branches, &remotes, "topic");
                let Pick::Track { upstream, name } = list.remote[0].pick.clone() else {
                    panic!("no local branch tracks origin/topic: {:?}", list.remote[0].pick)
                };
                assert_eq!((upstream.as_slice(), name.as_str()), (&b"refs/remotes/origin/topic"[..], "topic"));
                app.dispatch(Intent::Dialog(DialogKind::TrackBranch { upstream, name }));
                wait(&ctx, &mut app);
                assert_eq!(app.dialog.as_ref().unwrap().first, "topic", "the name is suggested");
                assert!(enabled(&ctx, &mut app, "dialog-submit"));
                app.dialog.as_mut().unwrap().first = home.clone();
                assert!(!enabled(&ctx, &mut app, "dialog-submit"), "an existing branch is never replaced");
                app.dialog.as_mut().unwrap().first = "topic".into();
                click(&ctx, &mut app, "dialog-submit");
                wait(&ctx, &mut app);
                let git = |args: &[&str]| String::from_utf8(f.run(args)).unwrap().trim().to_owned();
                assert_eq!(git(&["branch", "--show-current"]), "topic");
                assert_eq!(git(&["rev-parse", "HEAD"]), topic);
                assert_eq!(git(&["rev-parse", "--abbrev-ref", "topic@{upstream}"]), "origin/topic");
                // Now a local branch tracks it, so the switcher's remote row is that branch.
                let snapshot = app.tab().unwrap().snapshot.as_ref().unwrap();
                assert_eq!(listing(&snapshot.branches, &remotes, "topic").remote[0].pick, Pick::Stay);
            },
        );
    }
}
