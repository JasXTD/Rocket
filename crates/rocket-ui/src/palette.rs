//! Command palette (Ctrl+K): what you can do now, in one searchable list.
//!
//! With nothing typed it lists the commands that can run right now, grouped: Recent (run
//! from here this session), Suggested, Go to, Branches, Repository, Custom commands, View
//! and App; without a repository, Start, Recent repositories and App. Typing gives one
//! ranked list across groups with the match highlighted and the group named on the right.
//! A command that cannot run stays in that list with its reason instead of its shortcut,
//! and the arrow keys skip it. Entries reuse the menus' intents; nothing runs on hover.
use crate::{app::*, backend::Action, style::*};
use eframe::egui::{text::LayoutJob, *};
use rocket_git::{BranchKind, CommitActionRequest, CommitOperation, MailImportAction, RebaseAction};

/// Commands remembered from this session, newest first, and how many Recent shows.
const RECENT_KEEP: usize = 5;
const RECENT_SHOWN: usize = 3;
/// Branches listed before anything is typed; typing searches all of them.
const BRANCHES_SHOWN: usize = 5;
const ROW_HEIGHT: f32 = 36.0;

#[derive(Default)]
pub struct Palette {
    pub open: bool,
    query: String,
    /// Index into the rows on screen; moved to the first runnable row when needed.
    selected: usize,
    focus: bool,
    /// Keys of commands run from the palette this session, newest first.
    recent: Vec<String>,
}

impl Palette {
    pub fn show(&mut self) {
        self.open = true;
        self.query.clear();
        self.selected = 0;
        self.focus = true;
    }
    pub fn toggle(&mut self) {
        if self.open {
            self.open = false;
        } else {
            self.show();
        }
    }
}

pub(crate) enum Effect {
    Intent(Box<Intent>),
    Sidebar,
    Labels,
    Console,
    Activity,
    Guide,
    Zoom(f32),
    Theme(crate::theme::Choice),
}

struct Entry {
    /// Section while nothing is typed; sections appear in the order entries are listed.
    group: &'static str,
    image: Icon,
    label: String,
    /// Muted text after the label, such as the upstream or a repository's folder.
    detail: String,
    shortcut: String,
    /// Why the command cannot run now; empty when it can.
    reason: &'static str,
    /// Stays the same when the label changes (commit counts), for Recent.
    key: String,
    effect: Effect,
}

impl Entry {
    fn enabled(&self) -> bool {
        self.reason.is_empty()
    }
    fn detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = detail.into();
        self
    }
    fn key(mut self, key: impl Into<String>) -> Self {
        self.key = key.into();
        self
    }
    /// The group a search result names on its right.
    fn tag(&self) -> &'static str {
        match self.group {
            "Go to" => "View",
            "Branches" => "Branch",
            "Suggested" | "Recent repositories" => "Repository",
            "Start" => "App",
            "Custom commands" => "Custom command",
            "Extensions" => "Extension",
            other => other,
        }
    }
}

/// One command per theme, as in View → Appearance.
fn themes(group: &'static str) -> impl Iterator<Item = Entry> {
    crate::theme::Choice::ALL.into_iter().map(move |choice| {
        entry(group, Icon::Contrast, format!("{} theme", choice.label()), "", "", Effect::Theme(choice))
            .key(format!("theme:{}", choice.key()))
    })
}

fn entry(
    group: &'static str,
    image: Icon,
    label: impl Into<String>,
    shortcut: &'static str,
    reason: &'static str,
    effect: Effect,
) -> Entry {
    let label = command_label(&label.into()).to_owned();
    Entry {
        group,
        image,
        key: label.clone(),
        label,
        detail: String::new(),
        shortcut: shortcut.into(),
        reason,
        effect,
    }
}

/// The reason of the first unmet condition, or "" when the command can run.
fn unmet(checks: &[(bool, &'static str)]) -> &'static str {
    checks.iter().find(|(ok, _)| !ok).map_or("", |(_, why)| why)
}

/// "Pull", "Pull 1 commit" or "Pull 3 commits".
fn counted(verb: &str, commits: u64) -> String {
    match commits {
        0 => verb.to_owned(),
        1 => format!("{verb} 1 commit"),
        n => format!("{verb} {n} commits"),
    }
}

/// One line of the list: an entry, and the byte range of the search match in its label.
struct Row {
    entry: usize,
    hit: Option<(usize, usize)>,
}

/// A titled run of rows; while searching there is one untitled section of results.
struct Section {
    name: &'static str,
    note: String,
    rows: Vec<Row>,
}

/// What the palette lists for `query`. Nothing typed: runnable commands by group, Recent
/// first. Typed: every command whose label contains the text (or all its words), ranked
/// by where the match starts, runnable before unavailable.
fn sections(entries: &[Entry], recent: &[String], query: &str) -> Vec<Section> {
    let needle = query.trim().to_lowercase();
    if needle.is_empty() {
        let mut sections = Vec::new();
        let rows: Vec<Row> = recent
            .iter()
            .filter_map(|key| entries.iter().position(|e| e.key == *key && e.enabled()))
            .take(RECENT_SHOWN)
            .map(|entry| Row { entry, hit: None })
            .collect();
        if !rows.is_empty() {
            sections.push(Section {
                name: "Recent",
                note: "this session".into(),
                rows,
            });
        }
        let mut groups: Vec<&'static str> = Vec::new();
        for e in entries {
            if !groups.contains(&e.group) {
                groups.push(e.group);
            }
        }
        for group in groups {
            let mut rows: Vec<Row> = entries
                .iter()
                .enumerate()
                .filter(|(_, e)| e.group == group && e.enabled())
                .map(|(entry, _)| Row { entry, hit: None })
                .collect();
            let mut note = String::new();
            if group == "Branches" && rows.len() > BRANCHES_SHOWN {
                note = format!("{BRANCHES_SHOWN} of {} · type to find more", rows.len());
                rows.truncate(BRANCHES_SHOWN);
            }
            if !rows.is_empty() {
                sections.push(Section {
                    name: group,
                    note,
                    rows,
                });
            }
        }
        return sections;
    }
    let words: Vec<&str> = needle.split_whitespace().collect();
    // (rank, label length, row): better match first, then shorter, then list order.
    let mut hits: Vec<(u8, usize, Row)> = Vec::new();
    for (entry, e) in entries.iter().enumerate() {
        let lower = e.label.to_lowercase();
        let (rank, hit) = if let Some(at) = lower.find(&needle) {
            let word = at == 0 || matches!(lower[..at].chars().last(), Some(' ' | '/' | '-' | '_'));
            let end = at + needle.len();
            // Highlight only where lowercasing kept the label's byte layout.
            let exact = e.label.is_char_boundary(at)
                && e.label.is_char_boundary(end)
                && e.label.get(at..end).is_some_and(|s| s.to_lowercase() == needle);
            (if at == 0 { 0 } else if word { 1 } else { 2 }, exact.then_some((at, end)))
        } else if words.len() > 1 && words.iter().all(|w| lower.contains(w)) {
            (3, None)
        } else {
            continue;
        };
        let rank = rank + if e.enabled() { 0 } else { 10 };
        hits.push((rank, e.label.len(), Row { entry, hit }));
    }
    hits.sort_by_key(|(rank, length, row)| (*rank, *length, row.entry));
    vec![Section {
        name: "",
        note: String::new(),
        rows: hits.into_iter().map(|(_, _, row)| row).collect(),
    }]
}

/// The next (or previous) runnable row after `current`, wrapping around.
fn step(runnable: &[usize], current: usize, down: bool) -> usize {
    let Some(k) = runnable.iter().position(|&i| i == current) else {
        return runnable.first().copied().unwrap_or(current);
    };
    let n = runnable.len();
    runnable[if down { (k + 1) % n } else { (k + n - 1) % n }]
}

/// A shortcut's keys: "Ctrl+Shift+B" → Ctrl, Shift, B; "Ctrl++" → Ctrl, +.
fn keys(shortcut: &str) -> Vec<&str> {
    let mut keys: Vec<&str> = shortcut.split('+').filter(|k| !k.is_empty()).collect();
    if shortcut.ends_with("++") || shortcut == "+" {
        keys.push("+");
    }
    keys
}

/// A key drawn as a keycap, right-aligned at `right`; returns its left edge.
fn paint_keycap(painter: &Painter, right: f32, center_y: f32, key: &str) -> f32 {
    let galley = painter.layout_no_wrap(key.to_owned(), FontId::proportional(11.0), MUTED());
    let width = (galley.size().x + 10.0).max(18.0);
    let rect = Rect::from_min_max(pos2(right - width, center_y - 9.0), pos2(right, center_y + 9.0));
    painter.rect(
        rect,
        4.0,
        FIELD(),
        Stroke::new(1.0, EDGE_HOT()),
        StrokeKind::Inside,
    );
    painter.galley(rect.center() - galley.size() / 2.0, galley, MUTED());
    rect.left()
}

/// A section title, with an optional note such as "this session".
fn section_title(ui: &mut Ui, name: &str, note: &str) {
    ui.add_space(4.0);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::hover());
    let painter = ui.painter();
    let galley = painter.layout_no_wrap(name.to_uppercase(), semibold(11.0), HINT());
    let width = galley.size().x;
    painter.galley(pos2(rect.left() + 10.0, rect.center().y - galley.size().y / 2.0), galley, HINT());
    if !note.is_empty() {
        painter.text(
            pos2(rect.left() + 18.0 + width, rect.center().y),
            Align2::LEFT_CENTER,
            note,
            FontId::proportional(11.0),
            FAINT(),
        );
    }
}

/// One command: icon, label with the match highlighted, detail, then (while searching)
/// its group, and its keys or, when it cannot run, why.
fn draw_row(ui: &mut Ui, e: &Entry, hit: Option<(usize, usize)>, current: bool, searching: bool) -> Response {
    let enabled = e.enabled();
    // A command that cannot run is a disabled widget, for screen readers and automation.
    let (rect, response) = ui
        .scope(|ui| {
            if !enabled {
                ui.disable();
            }
            ui.allocate_exact_size(vec2(ui.available_width(), ROW_HEIGHT), Sense::click())
        })
        .inner;
    let painter = ui.painter_at(rect);
    if current {
        painter.rect_filled(rect, 8.0, SELECTED());
        painter.rect_filled(
            Rect::from_min_size(rect.min + vec2(0.0, 8.0), vec2(2.5, rect.height() - 16.0)),
            1.0,
            BLUE(),
        );
    } else if enabled && response.hovered() {
        painter.rect_filled(rect, 8.0, PALETTE_HOT());
    }
    let ink = if !enabled {
        FAINT()
    } else if current {
        TEXT()
    } else {
        TEXT_SOFT()
    };
    let icon_ink = if !enabled {
        DIM()
    } else if current {
        BLUE()
    } else {
        HINT()
    };
    icon(
        &painter,
        Rect::from_center_size(pos2(rect.left() + 20.0, rect.center().y), vec2(16.0, 16.0)),
        e.image,
        icon_ink,
    );
    // Right side first: keys or the reason, then the group while searching.
    let y = rect.center().y;
    let mut right = rect.right() - 10.0;
    if enabled {
        for key in keys(&e.shortcut).into_iter().rev() {
            right = paint_keycap(&painter, right, y, key) - 3.0;
        }
    } else {
        let galley = painter.layout_no_wrap(e.reason.to_owned(), FontId::proportional(12.0), HINT());
        right -= galley.size().x;
        painter.galley(pos2(right, y - galley.size().y / 2.0), galley, HINT());
    }
    if searching {
        let galley = painter.layout_no_wrap(e.tag().to_owned(), FontId::proportional(12.0), FAINT());
        right -= galley.size().x + 12.0;
        painter.galley(pos2(right, y - galley.size().y / 2.0), galley, FAINT());
    }
    // The label keeps most of the width; the detail takes what is left.
    let left = rect.left() + 40.0;
    let room = (right - 12.0 - left).max(0.0);
    let mut job = LayoutJob::default();
    let format = |font: FontId, color: Color32| TextFormat {
        font_id: font,
        color,
        ..Default::default()
    };
    let plain = FontId::proportional(13.5);
    match hit {
        Some((start, end)) => {
            job.append(&e.label[..start], 0.0, format(plain.clone(), ink));
            job.append(&e.label[start..end], 0.0, format(semibold(13.5), if enabled { BLUE() } else { ink }));
            job.append(&e.label[end..], 0.0, format(plain, ink));
        }
        None => job.append(&e.label, 0.0, format(plain, ink)),
    }
    job.wrap = text::TextWrapping::truncate_at_width(if e.detail.is_empty() { room } else { room * 0.7 });
    let label = painter.layout_job(job);
    let label_width = label.size().x;
    painter.galley(pos2(left, y - label.size().y / 2.0), label, ink);
    if !e.detail.is_empty() {
        let detail_left = left + label_width + 10.0;
        let mut job = LayoutJob::simple_singleline(e.detail.clone(), FontId::proportional(12.0), FAINT());
        job.wrap = text::TextWrapping::truncate_at_width((right - 12.0 - detail_left).max(0.0));
        let detail = painter.layout_job(job);
        painter.galley(pos2(detail_left, y - detail.size().y / 2.0), detail, FAINT());
    }
    let text = if enabled {
        e.label.clone()
    } else {
        format!("{} ({})", e.label, e.reason)
    };
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, enabled, &text));
    response
}

impl Rocket {
    /// One entry per enabled extension.
    fn extension_entries(&self, git: (bool, &'static str)) -> impl Iterator<Item = Entry> + '_ {
        self.integrations.entries().iter().map(move |e| {
            entry("Go to", Icon::Puzzle, e.name.clone(), "", unmet(&[git]), Effect::Intent(Box::new(Intent::Extension(e.id.clone()))))
                .key(format!("extension:{}", e.id))
        })
    }
    /// Every command of the enabled extensions, named after its extension. The row that
    /// clears what is in their way comes first, and the commands stay findable with their
    /// reason. Without a repository, commands that need one stay out.
    fn extension_commands(&self, tab: Option<&Tab>) -> Vec<Entry> {
        use crate::integrations::{CommandTarget, Fix};
        use rocket_integrations::Target;
        let busy = self.integrations.command_block(self.backend.is_some() && !self.interaction_busy());
        let waiting = (busy.is_none(), busy.unwrap_or(""));
        let mut list = Vec::new();
        for e in self.integrations.entries() {
            if e.commands.is_empty() {
                continue;
            }
            let aimed = e.commands.iter().any(|c| c.target.is_some());
            if let Some(fix) = e.block.or((aimed && !e.shares && tab.is_some()).then_some(Fix::Share)) {
                let image = if fix == Fix::Review { Icon::Warning } else { Icon::Puzzle };
                list.push(
                    entry("Extensions", image, fix.label(&e.name), "", unmet(&[waiting]), Effect::Intent(Box::new(Intent::ExtensionFix(e.id.clone(), fix))))
                        .key(format!("extension-fix:{}", e.id)),
                );
            }
            for command in &e.commands {
                let (target, needs) = match (command.target, tab) {
                    (None, _) => (None, true),
                    (Some(_), None) => continue,
                    (Some(Target::Commit), Some(tab)) => (tab.selected_commit.as_ref().map(|id| CommandTarget::commit(&tab.path, id)), false),
                    (Some(Target::Branch), Some(tab)) => {
                        let head = tab.snapshot.as_ref().and_then(|s| s.status.as_ref()).map(|s| &s.head);
                        let name = match head {
                            Some(rocket_git::Head::Branch { name, .. }) => Some(name),
                            _ => None,
                        };
                        (name.map(|name| CommandTarget::branch(&tab.path, name)), false)
                    }
                };
                let wanting = match command.target {
                    Some(Target::Commit) => "Select a commit first",
                    Some(Target::Branch) => "No branch is checked out",
                    None => "",
                };
                let detail = match target.as_ref().map(|t| &t.chosen) {
                    Some(crate::integrations::Chosen::Commit(id)) => id.chars().take(7).collect(),
                    Some(crate::integrations::Chosen::Branch(name)) => name.clone(),
                    None => String::new(),
                };
                let fix = e.fix_for(command.target);
                let checks = [waiting, (fix.is_none(), fix.map_or("", Fix::reason)), (needs || target.is_some(), wanting)];
                let run = e.run(command, target);
                list.push(
                    entry("Extensions", Icon::Puzzle, format!("{}: {}", e.name, command.label), "", unmet(&checks), Effect::Intent(Box::new(Intent::ExtensionCommand(run))))
                        .detail(detail)
                        .key(format!("extension-command:{}/{}", e.id, command.id)),
                );
            }
        }
        list
    }
    fn palette_entries(&self) -> Vec<Entry> {
        let go = |intent| Effect::Intent(Box::new(intent));
        let git = (self.backend.is_some(), "Git is not ready yet");
        let idle = (self.write.is_none(), "Wait for the current operation");
        let Some(tab) = self.tab() else {
            // No repository: start or reopen one, or change settings.
            let mut list = vec![
                entry("Start", Icon::Folder, "Open repository", "Ctrl+O", unmet(&[git]), go(Intent::Dialog(DialogKind::Open))),
                entry("Start", Icon::Fetch, "Clone repository", "Ctrl+Shift+O", unmet(&[git, idle]), go(Intent::Dialog(DialogKind::Clone))),
                entry("Start", Icon::Plus, "New repository", "Ctrl+N", unmet(&[git, idle]), go(Intent::Dialog(DialogKind::NewRepository))),
                entry("Start", Icon::Database, "Repository manager", "Ctrl+Shift+M", unmet(&[git, idle]), go(Intent::Dialog(DialogKind::RepositoryManager))),
                entry("Start", Icon::Info, "Setup Guide", "", unmet(&[idle]), go(Intent::GuidedSetup)),
            ];
            for path in &self.recent {
                let full = crate::views::display_path(path);
                let name = path
                    .file_name()
                    .map_or_else(|| full.clone(), |n| n.to_string_lossy().into_owned());
                list.push(
                    entry("Recent repositories", Icon::Folder, format!("Open {name}"), "", unmet(&[git]), go(Intent::Open(path.clone())))
                        .detail(full)
                        .key(format!("open:{}", path.display())),
                );
            }
            list.extend([
                entry("App", Icon::Sliders, "Preferences", "Ctrl+,", "", go(Intent::Dialog(DialogKind::Preferences))),
                entry("App", Icon::Info, "Guide", "F1", "", Effect::Guide),
                entry("App", Icon::Keyboard, "Shortcuts", "", "", go(Intent::Preferences(crate::preferences::Page::Shortcuts))),
                entry("App", Icon::Prompt, "Terminal settings", "", "", go(Intent::Preferences(crate::preferences::Page::Terminal))),
                entry("App", Icon::User, "Accounts", "", "", go(Intent::Preferences(crate::preferences::Page::Accounts))),
                entry("App", Icon::Key, "SSH keys", "", "", go(Intent::Preferences(crate::preferences::Page::SshKeys))),
                entry("App", Icon::Console, "Edit custom commands", "", unmet(&[idle]), go(Intent::Preferences(crate::preferences::Page::Commands))),
                entry("App", Icon::Puzzle, "Extensions", "", "", go(Intent::Preferences(crate::preferences::Page::Integrations))),
                entry("App", Icon::Pull, "Install extension from file", "", unmet(&[git]), go(Intent::InstallExtension)),
                entry("App", Icon::Console, "Lua playground", "", unmet(&[git]), go(Intent::Playground)),
                entry("App", Icon::Search, "Zoom in", "Ctrl++", "", Effect::Zoom(0.1)),
                entry("App", Icon::Search, "Zoom out", "Ctrl+−", "", Effect::Zoom(-0.1)),
            ]);
            list.extend(self.extension_entries(git));
            list.extend(self.extension_commands(None));
            list.extend(themes("App"));
            return list;
        };
        let snapshot = tab.snapshot.as_ref();
        let status = snapshot.and_then(|s| s.status.as_ref());
        let (ahead, behind) = status.and_then(|s| s.ahead_behind).unwrap_or((0, 0));
        let upstream = status.and_then(|s| s.upstream.as_deref()).map(short_ref);
        let remotes = (snapshot.is_some_and(|s| !s.remotes.is_empty()), "Add a remote first");
        let write_reason = if self.quitting {
            "Rocket is closing"
        } else if self.write.is_some() {
            "Wait for the current operation"
        } else if tab.loading() {
            "Wait for the repository to load"
        } else if !tab.worktree() {
            "Needs a working tree"
        } else {
            "Not available right now"
        };
        let write = (self.can_write(), write_reason);
        let settled = (
            snapshot
                .and_then(|s| s.state.as_ref())
                .is_some_and(|s| s.is_idle()),
            "Finish the operation in progress first",
        );
        let free = (!self.busy(), "Wait for the current work to finish");
        let calm = (!self.interaction_busy(), "Wait for the current work to finish");

        let mut list = vec![
            entry("Repository", Icon::Commit, "Undo last commit", "", unmet(&[write, (tab.undo_commit.is_some(), "No ordinary Rocket commit is available to undo in this session")]), go(Intent::Dialog(DialogKind::UndoCommit))),
            entry("App", Icon::Info, "Setup Guide", "", unmet(&[idle]), go(Intent::GuidedSetup)),
            entry("Suggested", Icon::Pull, counted("Pull", behind), "", unmet(&[write]), go(Intent::Dialog(DialogKind::Pull)))
                .detail(upstream.as_ref().map_or_else(String::new, |u| format!("from {u}")))
                .key("pull"),
            entry("Suggested", Icon::Push, counted("Push", ahead), "", unmet(&[write, remotes]), go(Intent::Dialog(DialogKind::Push)))
                .detail(upstream.as_ref().map_or_else(String::new, |u| format!("to {u}")))
                .key("push"),
            entry("Suggested", Icon::Branch, "New branch", "Ctrl+Shift+B", unmet(&[write]), go(Intent::Dialog(DialogKind::NewBranch))),
            entry("Suggested", Icon::Refresh, "Refresh repository", "F5", unmet(&[idle]), go(Intent::Refresh)),
        ];
        for (label, view, image) in [
            ("Repository summary", View::Summary, Icon::Folder),
            ("Changes", View::Changes, Icon::File),
            ("Commits", View::Commits, Icon::Commit),
            ("Compare commits and branches", View::Compare, Icon::Columns),
            ("Files", View::Tree, Icon::Folder),
            ("Branches", View::Branches, Icon::Branch),
            ("Remotes", View::Remotes, Icon::Remote),
            ("Stashes", View::Stashes, Icon::Stash),
            ("File history", View::FileHistory, Icon::Clock),
            ("AI branch review", View::BranchReview, Icon::Search),
        ] {
            list.push(entry("Go to", image, label, "", "", go(Intent::View(view))).key(format!("view:{label}")));
        }
        list.extend(self.extension_entries(git));
        if let Some(snapshot) = snapshot {
            for branch in snapshot
                .branches
                .iter()
                .filter(|b| b.kind == BranchKind::Local && !b.is_head)
            {
                let name = branch
                    .reference
                    .strip_prefix(b"refs/heads/")
                    .unwrap_or(&branch.reference)
                    .to_vec();
                let short = short_ref(&branch.reference);
                list.push(
                    entry("Branches", Icon::Branch, format!("Switch to {short}"), "", unmet(&[write]), go(Intent::Execute(tab.id, Action::SwitchBranch(name))))
                        .key(format!("branch:{short}")),
                );
            }
        }
        let commit_action = |operation, targets| {
            go(Intent::Dialog(DialogKind::CommitAction(CommitActionRequest {
                operation,
                targets,
                mainline: None,
                recovery: None,
            })))
        };
        let selected = |verb: &str, n: usize| match n {
            0 => verb.to_owned(),
            1 => format!("{verb} selected commit"),
            n => format!("{verb} {n} selected commits"),
        };
        let (picks, reverts) = (
            tab.commit_targets(CommitOperation::CherryPick),
            tab.commit_targets(CommitOperation::Revert),
        );
        let (pick_label, revert_label) = (selected("Cherry-pick", picks.len()), selected("Revert", reverts.len()));
        let commits = |n: usize| (n > 0, "Select commits first");
        list.extend([
            entry("Repository", Icon::Fetch, "Fetch", "", unmet(&[idle, remotes]), go(Intent::Dialog(DialogKind::Fetch))),
            entry("Repository", Icon::Merge, "Merge branch", "", unmet(&[write]), go(Intent::Dialog(DialogKind::Merge))),
            entry("Repository", Icon::Branch, "Rebase", "", unmet(&[write, settled]), go(Intent::Dialog(DialogKind::Rebase(RebaseAction::Start)))),
            entry("Repository", Icon::Branch, "Interactive rebase", "", unmet(&[write, settled]), go(Intent::Dialog(DialogKind::Rebase(RebaseAction::Interactive)))),
            entry("Repository", Icon::Stash, "Save changes to stash", "", unmet(&[write]), go(Intent::Dialog(DialogKind::Stash))),
            entry("Repository", Icon::Commit, "Amend latest commit", "", unmet(&[write, settled]), commit_action(CommitOperation::Amend, Vec::new())),
            entry("Repository", Icon::Commit, pick_label, "", unmet(&[write, settled, commits(picks.len())]), commit_action(CommitOperation::CherryPick, picks))
                .key("cherry-pick"),
            entry("Repository", Icon::Commit, revert_label, "", unmet(&[write, settled, commits(reverts.len())]), commit_action(CommitOperation::Revert, reverts))
                .key("revert"),
            entry("Repository", Icon::Branch, "Delete branches", "", unmet(&[write]), go(Intent::Dialog(DialogKind::DeleteBranches))),
        ]);
        {
            use crate::gitflow::Request;
            use rocket_git::GitFlowKind as Kind;
            list.extend(
                [
                    ("Git Flow: Initialize", Request::Init),
                    ("Git Flow: Start feature", Request::Start(Kind::Feature)),
                    ("Git Flow: Start release", Request::Start(Kind::Release)),
                    ("Git Flow: Start hotfix", Request::Start(Kind::Hotfix)),
                    ("Git Flow: Start support branch", Request::Start(Kind::Support)),
                    ("Git Flow: Finish", Request::Finish),
                ]
                .map(|(label, request)| {
                    entry("Repository", Icon::Branch, label, "", unmet(&[write]), go(Intent::Dialog(DialogKind::GitFlow(request))))
                }),
            );
        }
        list.extend([
            entry("Repository", Icon::Tag, "Tags", "", unmet(&[free]), go(Intent::Dialog(DialogKind::Tags))),
            entry("Repository", Icon::Worktree, "Worktrees", "", unmet(&[free]), go(Intent::Dialog(DialogKind::Worktrees))),
            entry("Repository", Icon::Modules, "Submodules", "", unmet(&[free]), go(Intent::Dialog(DialogKind::Submodules))),
            entry("Repository", Icon::Database, "Git LFS", "", unmet(&[free]), go(Intent::Dialog(DialogKind::Lfs))),
            entry("Repository", Icon::Branch, "Branch workflows & cleanup", "", unmet(&[write]), go(Intent::Dialog(DialogKind::Workflows))),
            entry("Repository", Icon::Refresh, "Reflog & recovery", "", unmet(&[free]), go(Intent::Dialog(DialogKind::Recovery))),
            entry("Repository", Icon::File, "File backups & restore", "", unmet(&[free]), go(Intent::Dialog(DialogKind::FileEdit(None)))),
            entry("Repository", Icon::Search, "Bisect", "", unmet(&[free]), go(Intent::Dialog(DialogKind::Bisect))),
            entry("Repository", Icon::Folder, "Browse files at commit", "", unmet(&[calm]), go(Intent::Tree(crate::tree::TreeIntent::OpenSelected))),
            entry("Repository", Icon::File, "Import patch", "", unmet(&[free]), go(Intent::Dialog(DialogKind::Patch(crate::patches::Mode::Import)))),
            entry("Repository", Icon::File, "Apply patch from clipboard", "Ctrl+V", unmet(&[free]), go(Intent::PastePatch)),
            entry("Repository", Icon::File, "Export patch", "", unmet(&[free]), go(Intent::Dialog(DialogKind::Patch(crate::patches::Mode::Export)))),
            entry("Repository", Icon::Commit, "Import commit series", "", unmet(&[write, free]), go(Intent::Dialog(DialogKind::MailImport(MailImportAction::Start)))),
            entry("Repository", Icon::File, "Export commit series", "", unmet(&[free]), go(Intent::ExportSeries(None))),
            entry("Repository", Icon::Activity, "Statistics", "", unmet(&[free]), go(Intent::Dialog(DialogKind::Statistics))),
            entry("Repository", Icon::Remote, "Hosting", "", "", go(Intent::Dialog(DialogKind::Hosting))),
            entry("Repository", Icon::Remote, "GitHub repositories & notifications", "", "", go(Intent::Dialog(DialogKind::GitHub))),
            entry("Repository", Icon::Remote, "Add remote", "", unmet(&[idle]), go(Intent::Dialog(DialogKind::AddRemote))),
            entry("Repository", Icon::Sliders, "Repository settings", "", unmet(&[idle]), go(Intent::Dialog(DialogKind::RepositorySettings))),
        ]);
        // Only while something is paused, where the banner offers the same three.
        let state = snapshot.and_then(|s| s.state.as_ref());
        if state.is_some_and(|s| s.mail_import) {
            for (label, action) in [
                ("Continue import", MailImportAction::Continue),
                ("Skip patch", MailImportAction::Skip),
                ("Abort import", MailImportAction::Abort),
            ] {
                list.push(entry("Repository", Icon::Commit, label, "", unmet(&[write]), go(Intent::Dialog(DialogKind::MailImport(action)))));
            }
        } else if state.is_some_and(|s| s.rebase) {
            for (label, action) in [
                ("Continue rebase", RebaseAction::Continue),
                ("Skip commit", RebaseAction::Skip),
                ("Abort rebase", RebaseAction::Abort),
            ] {
                list.push(entry("Repository", Icon::Branch, label, "", unmet(&[write]), go(Intent::Dialog(DialogKind::Rebase(action)))));
            }
        } else if let Some(crate::chrome::Recover::Commit { operation, skip }) = state.and_then(|s| crate::chrome::Recover::of(s, snapshot.and_then(|t| t.sequence.as_ref()))) {
            let word = operation.label();
            let ways = [
                Some((format!("Continue {word}"), rocket_git::CommitRecovery::Continue)),
                skip.filter(|&stopped| stopped).map(|_| ("Skip commit".to_owned(), rocket_git::CommitRecovery::Skip)),
                Some((format!("Abort {word}"), rocket_git::CommitRecovery::Abort)),
            ];
            for (label, recovery) in ways.into_iter().flatten() {
                let request = rocket_git::CommitActionRequest { operation, targets: Vec::new(), mainline: None, recovery: Some(recovery) };
                list.push(entry("Repository", Icon::Commit, label, "", unmet(&[write]), go(Intent::Dialog(DialogKind::CommitAction(request)))));
            }
        }
        for (i, command) in self.commands.iter().enumerate().filter(|(_, c)| c.scope == rocket_git::CommandScope::Repository) {
            let name = command.name.trim();
            list.push(
                entry("Custom commands", Icon::Console, format!("Run {}", rocket_git::custom_command_display(name)), "", unmet(&[write]), go(Intent::Dialog(DialogKind::RunCommand(i))))
                    .key(format!("command:{name}")),
            );
        }
        list.extend(self.extension_commands(Some(tab)));
        list.extend([
            entry("View", Icon::Activity, "Toggle activity", "Ctrl+`", "", Effect::Console),
            entry("View", Icon::Console, "Toggle terminal", "Ctrl+Shift+`", "", go(Intent::Terminal)),
            entry("View", Icon::Layout, "Toggle sidebar", "Ctrl+B", "", Effect::Sidebar),
            entry("View", Icon::Layout, "Toggle sidebar labels", "", "", Effect::Labels),
            entry("View", Icon::Activity, "Activity & diagnostics", "", "", Effect::Activity),
            entry("View", Icon::Commit, "Collapse all merges", "", "", go(Intent::FoldAllMerges(true))),
            entry("View", Icon::Commit, "Expand all merges", "", "", go(Intent::FoldAllMerges(false))),
            entry("View", Icon::Search, "Zoom in", "Ctrl++", "", Effect::Zoom(0.1)),
            entry("View", Icon::Search, "Zoom out", "Ctrl+−", "", Effect::Zoom(-0.1)),
            entry("App", Icon::Folder, "Open repository", "Ctrl+O", unmet(&[git]), go(Intent::Dialog(DialogKind::Open))),
            entry("App", Icon::Fetch, "Clone repository", "Ctrl+Shift+O", unmet(&[git, idle]), go(Intent::Dialog(DialogKind::Clone))),
            entry("App", Icon::Plus, "New repository", "Ctrl+N", unmet(&[git, idle]), go(Intent::Dialog(DialogKind::NewRepository))),
            entry("App", Icon::Database, "Repository manager", "Ctrl+Shift+M", unmet(&[git, idle]), go(Intent::Dialog(DialogKind::RepositoryManager))),
            entry("App", Icon::Close, "Close repository", "Ctrl+W", "", go(Intent::CloseTab(tab.id))),
            entry("App", Icon::Console, "Edit custom commands", "", unmet(&[idle]), go(Intent::Preferences(crate::preferences::Page::Commands))),
            entry("App", Icon::Sliders, "Preferences", "Ctrl+,", "", go(Intent::Dialog(DialogKind::Preferences))),
            entry("App", Icon::Info, "Guide", "F1", "", Effect::Guide),
            entry("App", Icon::Keyboard, "Shortcuts", "", "", go(Intent::Preferences(crate::preferences::Page::Shortcuts))),
            entry("App", Icon::Prompt, "Terminal settings", "", "", go(Intent::Preferences(crate::preferences::Page::Terminal))),
            entry("App", Icon::User, "Accounts", "", "", go(Intent::Preferences(crate::preferences::Page::Accounts))),
            entry("App", Icon::Key, "SSH keys", "", "", go(Intent::Preferences(crate::preferences::Page::SshKeys))),
            entry("App", Icon::Puzzle, "Extensions", "", "", go(Intent::Preferences(crate::preferences::Page::Integrations))),
            entry("App", Icon::Pull, "Install extension from file", "", unmet(&[git]), go(Intent::InstallExtension)),
            entry("App", Icon::Console, "Lua playground", "", unmet(&[git]), go(Intent::Playground)),
        ]);
        list.extend(themes("View"));
        list
    }

    pub(crate) fn command_palette(&mut self, ctx: &Context, intents: &mut Vec<Intent>) {
        if !self.palette.open {
            return;
        }
        let mut entries = self.palette_entries();
        for entry in &mut entries {
            use crate::shortcuts::model::Action;
            let action = if entry.shortcut.is_empty() && let Effect::Intent(intent) = &entry.effect { match intent.as_ref() {
                Intent::Dialog(DialogKind::Fetch) => Some(Action::Fetch),
                Intent::Dialog(DialogKind::Pull) => Some(Action::Pull),
                Intent::Dialog(DialogKind::Push) => Some(Action::Push),
                Intent::View(View::Changes) => Some(Action::Changes),
                Intent::View(View::Commits) => Some(Action::History),
                _ => None,
            }} else {None};
            entry.shortcut = action.map_or_else(||crate::shortcuts::hint(ctx, &entry.shortcut), |a|self.keybindings.bindings.label(a));
        }
        let searching = !self.palette.query.trim().is_empty();
        let sections = sections(&entries, &self.palette.recent, &self.palette.query);
        let rows: Vec<&Row> = sections.iter().flat_map(|s| &s.rows).collect();
        let runnable: Vec<usize> = (0..rows.len())
            .filter(|&i| entries[rows[i].entry].enabled())
            .collect();
        let (down, up, enter) = ctx.input_mut(|i| {
            (
                i.consume_key(Modifiers::NONE, Key::ArrowDown),
                i.consume_key(Modifiers::NONE, Key::ArrowUp),
                i.consume_key(Modifiers::NONE, Key::Enter),
            )
        });
        // usize::MAX selects nothing: no row can run, or the query just changed.
        let mut selected = self.palette.selected;
        if !runnable.contains(&selected) {
            selected = runnable.first().copied().unwrap_or(usize::MAX);
        }
        if down || up {
            selected = step(&runnable, selected, down);
        }
        let mut run = (enter && runnable.contains(&selected)).then_some(selected);
        let width = (ctx.content_rect().width() - 32.0).clamp(280.0, 640.0);
        let repository = self.tab().is_some();
        let status = if searching {
            match rows.len() {
                1 => "1 match".to_owned(),
                n => format!("{n} matches"),
            }
        } else if repository {
            "Type to search every command, branch and view".to_owned()
        } else {
            "Repository commands appear once a repository is open".to_owned()
        };
        let palette = &mut self.palette;
        let widgets = &mut self.frame_widgets;
        let modal = Modal::new(Id::new("command-palette"))
            .area(Modal::default_area(Id::new("command-palette")).anchor(Align2::CENTER_TOP, vec2(0.0, 84.0)))
            .backdrop_color(Color32::from_black_alpha(110))
            .frame(crate::pages::modal_frame())
            .show(ctx, |ui| {
                ui.set_width(width);
                ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                // Search: what to look for, and Esc to close.
                ui.allocate_ui_with_layout(vec2(width, 54.0), Layout::left_to_right(Align::Center), |ui| {
                    ui.add_space(16.0);
                    let (rect, _) = ui.allocate_exact_size(vec2(17.0, 17.0), Sense::hover());
                    icon(ui.painter(), rect, Icon::Search, HINT());
                    ui.add_space(12.0);
                    let hint = if repository {
                        "Search commands, branches and views"
                    } else {
                        "Search commands and recent repositories"
                    };
                    let edit = ui.add(
                        TextEdit::singleline(&mut palette.query)
                            .id(Id::new("command-palette-input"))
                            .frame(Frame::NONE)
                            .font(FontId::proportional(15.0))
                            .hint_text(hint)
                            .desired_width((ui.available_width() - 56.0).max(40.0)),
                    );
                    widgets.insert("palette-input".into(), (&edit).into());
                    if palette.focus {
                        edit.request_focus();
                        palette.focus = false;
                    }
                    if edit.changed() {
                        // The list follows next frame; start again at its first runnable row.
                        selected = usize::MAX;
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.add_space(14.0);
                        crate::preferences::keycap(ui, "Esc");
                    });
                });
                let line = ui.allocate_exact_size(vec2(width, 1.0), Sense::hover()).0;
                ui.painter().rect_filled(line, 0.0, HAIRLINE());
                ScrollArea::vertical()
                    .id_salt("command-palette-list")
                    .max_height(420.0)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        Frame::new().inner_margin(6).show(ui, |ui| {
                            if rows.is_empty() && searching {
                                ui.add_space(28.0);
                                ui.vertical_centered(|ui| {
                                    ui.label(
                                        RichText::new(format!(
                                            "No command, branch or view matches “{}”.",
                                            palette.query.trim()
                                        ))
                                        .color(HINT()),
                                    );
                                });
                                ui.add_space(28.0);
                            }
                            // The pointer selects only when it moves, so a mouse resting
                            // over the list does not undo the arrow keys.
                            let pointer_moved = ui.input(|i| i.pointer.delta() != Vec2::ZERO);
                            let mut index = 0;
                            for section in &sections {
                                if !section.name.is_empty() {
                                    section_title(ui, section.name, &section.note);
                                }
                                for row in &section.rows {
                                    let e = &entries[row.entry];
                                    let current = index == selected;
                                    let response = draw_row(ui, e, row.hit, current, searching);
                                    widgets.insert(format!("palette-row-{index}"), (&response).into());
                                    if current && (down || up) {
                                        response.scroll_to_me(None);
                                    }
                                    if e.enabled() {
                                        if response.hovered() && pointer_moved && selected != usize::MAX {
                                            selected = index;
                                        }
                                        if response.clicked() {
                                            run = Some(index);
                                        }
                                    }
                                    index += 1;
                                }
                            }
                        });
                    });
                let line = ui.allocate_exact_size(vec2(width, 1.0), Sense::hover()).0;
                ui.painter().rect_filled(line, 0.0, HAIRLINE());
                // Footer: the keys, then how many matched or what to type.
                Frame::new()
                    .fill(SUNKEN())
                    .inner_margin(Margin::symmetric(14, 0))
                    .show(ui, |ui| {
                        ui.allocate_ui_with_layout(vec2(width - 28.0, 38.0), Layout::left_to_right(Align::Center), |ui| {
                            ui.spacing_mut().item_spacing.x = 5.0;
                            for (keys, action) in [(&["↑", "↓"][..], "Move"), (&["↵"][..], "Run"), (&["Esc"][..], "Close")] {
                                for key in keys {
                                    crate::preferences::keycap(ui, key);
                                }
                                ui.label(RichText::new(action).size(12.0).color(HINT()));
                                ui.add_space(9.0);
                            }
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                ui.add(Label::new(RichText::new(&status).size(12.0).color(HINT())).truncate());
                            });
                        });
                    });
            });
        self.palette.selected = selected;
        if modal.should_close() {
            self.palette.open = false;
            return;
        }
        let Some(index) = run.filter(|i| runnable.contains(i)) else {
            return;
        };
        let entry = entries.swap_remove(rows[index].entry);
        self.palette.open = false;
        self.palette.recent.retain(|key| *key != entry.key);
        self.palette.recent.insert(0, entry.key);
        self.palette.recent.truncate(RECENT_KEEP);
        match entry.effect {
            Effect::Intent(intent) => intents.push(*intent),
            Effect::Sidebar => self.show_sidebar = !self.show_sidebar,
            Effect::Labels => self.toggle_sidebar_labels(ctx),
            Effect::Console => self.toggle_activity(),
            Effect::Activity => {
                self.activity_history.show();
                self.external.open = false;
            }
            Effect::Guide => crate::guide::request(ctx),
            Effect::Zoom(delta) => {
                ctx.set_zoom_factor((ctx.zoom_factor() + delta).clamp(0.75, 1.5))
            }
            Effect::Theme(choice) => self.theme.choice = choice,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_common as common;
    use std::{
        thread,
        time::{Duration, Instant},
    };

    impl Rocket {
        /// The palette's rows for one group: label, detail and why it can't run.
        pub(crate) fn palette_rows(&self, group: &str) -> Vec<(String, String, &'static str)> {
            self.palette_entries()
                .into_iter()
                .filter(|e| e.group == group)
                .map(|e| (e.label, e.detail, e.reason))
                .collect()
        }
    }

    fn sample() -> Vec<Entry> {
        vec![
            entry("Suggested", Icon::Pull, "Pull 2 commits", "", "", Effect::Sidebar).key("pull"),
            entry("Suggested", Icon::Push, "Push", "", "Add a remote first", Effect::Sidebar).key("push"),
            entry("Go to", Icon::File, "Changes", "", "", Effect::Sidebar),
            entry("Repository", Icon::Branch, "Rebase", "", "", Effect::Sidebar),
            entry("Repository", Icon::Branch, "Interactive rebase", "", "", Effect::Sidebar),
            entry("Repository", Icon::Commit, "Revert", "", "Select commits first", Effect::Sidebar).key("revert"),
            entry("Repository", Icon::Refresh, "Reflog & recovery", "", "", Effect::Sidebar),
            entry("Suggested", Icon::Branch, "New branch", "Ctrl+Shift+B", "", Effect::Sidebar),
        ]
    }

    /// The series commands are always there; Continue, Skip and Abort only while paused.
    #[test]
    fn series_commands_are_found_and_paused_ones_follow_the_pause() {
        common::isolated(
            "ROCKET_PALETTE_SERIES_CASE",
            "palette::tests::series_commands_are_found_and_paused_ones_follow_the_pause",
            || {
                let f = common::Fixture::new();
                f.write("a.txt", "a\n");
                f.commit("base");
                let ctx = Context::default();
                let mut app = crate::batch::flow_tests::open(&ctx, &f);
                let names = |app: &Rocket| app.palette_rows("Repository").into_iter().map(|r| r.0).collect::<Vec<_>>();
                let idle = names(&app);
                assert!(idle.iter().any(|n| n == "Import commit series"), "{idle:?}");
                assert!(idle.iter().any(|n| n == "Export commit series"), "{idle:?}");
                assert!(!idle.iter().any(|n| n == "Continue import" || n == "Abort rebase"), "{idle:?}");
                let state = app.tabs[0].snapshot.as_mut().unwrap().state.as_mut().unwrap();
                state.mail_import = true;
                let paused = names(&app);
                assert!(["Continue import", "Skip patch", "Abort import"].iter().all(|l| paused.iter().any(|n| n == l)), "{paused:?}");
                let state = app.tabs[0].snapshot.as_mut().unwrap().state.as_mut().unwrap();
                state.mail_import = false;
                state.rebase = true;
                let paused = names(&app);
                assert!(["Continue rebase", "Skip commit", "Abort rebase"].iter().all(|l| paused.iter().any(|n| n == l)), "{paused:?}");
                let state = app.tabs[0].snapshot.as_mut().unwrap().state.as_mut().unwrap();
                state.rebase = false;
                state.revert = true;
                let paused = names(&app);
                assert!(["Continue revert", "Abort revert"].iter().all(|l| paused.iter().any(|n| n == l)), "{paused:?}");
                assert!(!paused.iter().any(|n| n == "Skip commit"), "a lone revert has nothing to skip to: {paused:?}");
            },
        );
    }

    fn labels<'a>(entries: &'a [Entry], section: &Section) -> Vec<&'a str> {
        section.rows.iter().map(|r| entries[r.entry].label.as_str()).collect()
    }

    #[test]
    fn nothing_typed_lists_what_can_run_by_group_with_recent_first() {
        let entries = sample();
        let recent = ["revert".to_owned(), "Rebase".to_owned(), "pull".to_owned()];
        let shown = sections(&entries, &recent, "  ");
        let names: Vec<_> = shown.iter().map(|s| s.name).collect();
        assert_eq!(names, ["Recent", "Suggested", "Go to", "Repository"]);
        assert_eq!(shown[0].note, "this session");
        // Revert cannot run, so Recent skips it; the counted Pull keeps its key.
        assert_eq!(labels(&entries, &shown[0]), ["Rebase", "Pull 2 commits"]);
        assert_eq!(labels(&entries, &shown[1]), ["Pull 2 commits", "New branch"], "Push cannot run");
        assert!(!labels(&entries, &shown[3]).contains(&"Revert"));
    }

    #[test]
    fn a_long_branch_list_waits_for_a_search() {
        let entries: Vec<Entry> = (0..8)
            .map(|i| entry("Branches", Icon::Branch, format!("Switch to topic-{i}"), "", "", Effect::Sidebar))
            .collect();
        let shown = sections(&entries, &[], "");
        assert_eq!(shown[0].rows.len(), BRANCHES_SHOWN);
        assert_eq!(shown[0].note, "5 of 8 · type to find more");
        assert_eq!(labels(&entries, &sections(&entries, &[], "topic-7")[0]), ["Switch to topic-7"]);
    }

    #[test]
    fn typing_ranks_matches_and_puts_unavailable_ones_last() {
        let entries = sample();
        let found = sections(&entries, &[], "Re");
        assert_eq!(found.len(), 1);
        assert!(found[0].name.is_empty(), "results have no section title");
        let rows: Vec<_> = found[0]
            .rows
            .iter()
            .map(|r| (entries[r.entry].label.as_str(), r.hit))
            .collect();
        assert_eq!(
            rows,
            [
                ("Rebase", Some((0, 2))),
                ("Reflog & recovery", Some((0, 2))),
                ("Interactive rebase", Some((12, 14))),
                ("Revert", Some((0, 2))),
            ]
        );
        // Words in another order still find it, without a highlight.
        let found = sections(&entries, &[], "branch new");
        assert_eq!(labels(&entries, &found[0]), ["New branch"]);
        assert_eq!(found[0].rows[0].hit, None);
        assert!(sections(&entries, &[], "zzz")[0].rows.is_empty());
    }

    #[test]
    fn arrows_skip_rows_that_cannot_run_and_shortcuts_split_into_keys() {
        let runnable = [0, 2, 5];
        assert_eq!(step(&runnable, 0, true), 2);
        assert_eq!(step(&runnable, 5, true), 0);
        assert_eq!(step(&runnable, 0, false), 5);
        assert_eq!(step(&runnable, 3, true), 0, "from a row that cannot run, the first one that can");
        assert_eq!(keys("Ctrl+Shift+B"), ["Ctrl", "Shift", "B"]);
        assert_eq!(keys("Ctrl++"), ["Ctrl", "+"]);
        assert_eq!(keys("Ctrl+−"), ["Ctrl", "−"]);
        assert!(keys("").is_empty());
    }

    #[test]
    fn without_a_repository_it_offers_start_and_app_and_enter_runs_a_match() {
        common::isolated(
            "ROCKET_PALETTE_UI_CASE",
            "palette::tests::without_a_repository_it_offers_start_and_app_and_enter_runs_a_match",
            || {
                let fixture = common::Fixture::new();
                let ctx = Context::default();
                let mut app = Rocket::with_log_directory(&ctx, vec![], fixture.root.join("logs"));
                let deadline = Instant::now() + Duration::from_secs(40);
                while app.backend.is_none() || app.busy() {
                    app.poll(&ctx);
                    assert!(Instant::now() < deadline, "Git discovery settles");
                    thread::sleep(Duration::from_millis(10));
                }
                let entries = app.palette_entries();
                let shown = sections(&entries, &[], "");
                let names: Vec<_> = shown.iter().map(|s| s.name).collect();
                assert_eq!(names, ["Start", "App"], "no recent repositories in a fresh profile");
                assert_eq!(labels(&entries, &shown[0])[0], "Open repository");
                assert!(labels(&entries, &shown[0]).contains(&"Setup Guide"));
                for e in &entries {
                    assert!(matches!(e.group, "Start" | "Recent repositories" | "App"), "{} needs a repository", e.label);
                }

                app.palette.show();
                app.palette.query = "pref".into();
                let mut intents = Vec::new();
                draw(&ctx, &mut app, &mut intents, vec![]);
                draw(&ctx, &mut app, &mut intents, vec![]);
                assert!(app.frame_widgets.contains_key("palette-row-0"));
                draw(&ctx, &mut app, &mut intents, vec![enter()]);
                assert!(matches!(intents.as_slice(), [Intent::Dialog(DialogKind::Preferences)]));
                assert!(!app.palette.open, "running a command closes the palette");
                assert_eq!(app.palette.recent, ["Preferences"]);
            },
        );
    }

    #[test]
    fn a_command_that_cannot_run_is_disabled_and_enter_skips_it() {
        common::isolated(
            "ROCKET_PALETTE_UI_CASE",
            "palette::tests::a_command_that_cannot_run_is_disabled_and_enter_skips_it",
            || {
                let f = common::Fixture::new();
                f.write("a.txt", "base\n");
                f.commit("base");
                let ctx = Context::default();
                let mut app = Rocket::with_log_directory(&ctx, vec![f.repository.clone()], f.root.join("logs"));
                let deadline = Instant::now() + Duration::from_secs(40);
                while app.busy() || app.tab().is_none_or(|t| t.loading()) {
                    app.poll(&ctx);
                    assert!(Instant::now() < deadline, "the repository loads");
                    thread::sleep(Duration::from_millis(10));
                }
                // The fixture has no remote, so Push is the one match and cannot run.
                app.palette.show();
                app.palette.query = "push".into();
                let mut intents = Vec::new();
                draw(&ctx, &mut app, &mut intents, vec![]);
                draw(&ctx, &mut app, &mut intents, vec![]);
                assert!(!app.frame_widgets["palette-row-0"].enabled, "Push needs a remote");
                assert!(!app.frame_widgets.contains_key("palette-row-1"));
                assert_eq!(app.palette.selected, usize::MAX, "nothing runnable is selected");
                draw(&ctx, &mut app, &mut intents, vec![enter()]);
                assert!(intents.is_empty(), "Enter runs nothing");
                assert!(app.palette.open);
            },
        );
    }

    fn enter() -> Event {
        Event::Key {
            key: Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }
    }

    fn draw(ctx: &Context, app: &mut Rocket, intents: &mut Vec<Intent>, events: Vec<Event>) {
        app.frame_widgets.clear();
        let mut output = ctx.run_ui(
            RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 860.0))),
                events,
                ..Default::default()
            },
            |ui| {
                let ctx = ui.ctx().clone();
                app.command_palette(&ctx, intents);
            },
        );
        output.textures_delta.clear();
    }
}
