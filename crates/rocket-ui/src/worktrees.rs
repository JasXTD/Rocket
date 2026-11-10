//! Worktree browser with cancellable reads and immutable write previews.
use crate::{
    app::*,
    backend::{Action, Outcome, user_error},
    pages::{self, control},
    style::*,
};
use eframe::egui::*;
use rocket_git::*;
use crate::i18n::{tr, trf, trn};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};

#[derive(Default)]
pub struct DialogState {
    pub entries: Vec<WorktreeEntry>,
    pub selected: Option<PathBuf>,
    pub creating: bool,
    pub removing: bool,
    pub maintaining: Option<WorktreeMaintenance>,
    pub existing: bool,
    /// Create a new local branch that tracks `upstream`, a remote branch.
    pub remote: bool,
    pub upstream: String,
    pub branch: String,
    pub target: String,
    pub preview: Option<Arc<WorktreePreview>>,
    pub list_task: Option<ReadTask<Vec<WorktreeEntry>>>,
    pub task: Option<ReadTask<WorktreePreview>>,
    pub open_task: Option<ReadTask<PathBuf>>,
    pub applying: bool,
    /// Each listed worktree's changes and last commit, read after the list.
    pub statuses: Vec<WorktreeStatus>,
    status_task: Option<ReadTask<Vec<WorktreeStatus>>>,
    /// The folder Rocket suggested for a new worktree, until it is edited.
    pub suggested: Option<String>,
}
impl DialogState {
    pub fn running(&self) -> bool {
        self.list_task.is_some() || self.task.is_some() || self.open_task.is_some()
    }
    /// The rows' changes are being read; the page stays usable meanwhile.
    pub(crate) fn reading_statuses(&self) -> bool {
        self.status_task.is_some()
    }
    pub fn selected_entry(&self) -> Option<&WorktreeEntry> {
        self.entries
            .iter()
            .find(|e| Some(&e.path) == self.selected.as_ref())
    }
    fn invalidate(&mut self) {
        self.task = None;
        self.preview = None;
    }
}
impl Rocket {
    pub(crate) fn start_worktree_read(&mut self) {
        let Some(d) = &mut self.dialog else { return };
        if !matches!(d.kind, DialogKind::Worktrees) {
            return;
        }
        let s = d.worktrees.get_or_insert_with(Default::default);
        if s.applying {
            return;
        }
        d.error = None;
        s.list_task = None;
        s.open_task = None;
        s.invalidate();
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
        if let Some(kind) = s.maintaining {
            let Some(entry) = s.selected_entry().cloned() else {
                return;
            };
            let destination = PathBuf::from(&d.first);
            match ReadTask::spawn(move |cancel| {
                backend.worktree_maintenance_preview(&path, kind, &entry, &destination, &cancel)
            }) {
                Ok(t) => s.task = Some(t),
                Err(e) => d.error = Some(user_error(&e)),
            }
        } else if s.creating {
            let destination = PathBuf::from(&d.first);
            let target = s.target.clone();
            let checkout = if s.existing {
                WorktreeCheckout::ExistingBranch(s.branch.clone())
            } else if s.remote {
                WorktreeCheckout::TrackingBranch {
                    branch: s.branch.clone(),
                    upstream: s.upstream.trim().to_owned(),
                }
            } else {
                WorktreeCheckout::NewBranch(s.branch.clone())
            };
            match ReadTask::spawn(move |cancel| {
                backend.worktree_create_preview(&path, &destination, checkout, &target, &cancel)
            }) {
                Ok(t) => s.task = Some(t),
                Err(e) => d.error = Some(user_error(&e)),
            }
        } else if s.removing {
            let Some(entry) = s.selected_entry().cloned() else {
                return;
            };
            match ReadTask::spawn(move |cancel| {
                backend.worktree_remove_preview(&path, &entry, &cancel)
            }) {
                Ok(t) => s.task = Some(t),
                Err(e) => d.error = Some(user_error(&e)),
            }
        } else {
            match ReadTask::spawn(move |cancel| backend.worktrees(&path, &cancel)) {
                Ok(t) => s.list_task = Some(t),
                Err(e) => d.error = Some(user_error(&e)),
            }
        }
    }
    pub(crate) fn poll_worktree_read(&mut self) {
        let active = self.tab().map(|t| t.id);
        let Some(d) = &mut self.dialog else { return };
        let Some(s) = &mut d.worktrees else { return };
        let mut read_statuses = false;
        if let Some(r) = s.status_task.as_mut().and_then(ReadTask::try_finish) {
            s.status_task = None;
            match r {
                Ok(statuses) => s.statuses = statuses,
                // The list itself still stands; the rows just lack their changes.
                Err(e) => d.error = Some(user_error(&e)),
            }
        }
        if let Some(r) = s.list_task.as_mut().and_then(ReadTask::try_finish) {
            s.list_task = None;
            match r {
                Ok(entries) => {
                    if !entries.iter().any(|e| Some(&e.path) == s.selected.as_ref()) {
                        s.selected = None;
                    }
                    s.entries = entries;
                    read_statuses = true;
                }
                Err(e) => {
                    s.entries.clear();
                    s.selected = None;
                    d.error = Some(user_error(&e));
                }
            }
        }
        if let Some(r) = s.task.as_mut().and_then(ReadTask::try_finish) {
            s.task = None;
            match r {
                Ok(p) => s.preview = Some(Arc::new(p)),
                Err(e) => d.error = Some(user_error(&e)),
            }
        }
        let mut opened = None;
        if let Some(r) = s.open_task.as_mut().and_then(ReadTask::try_finish) {
            s.open_task = None;
            match r {
                Ok(path) => opened = Some(path),
                Err(e) => d.error = Some(user_error(&e)),
            }
        }
        let same_owner = d.owner == active;
        if read_statuses {
            self.start_worktree_statuses();
        }
        if let Some(path) = opened
            && same_owner
        {
            self.dialog = None;
            let index = self.tabs.iter().position(|t| {
                t.snapshot
                    .as_ref()
                    .and_then(|s| s.info.worktree.as_ref())
                    .unwrap_or(&t.path)
                    == &path
            });
            if let Some(i) = index {
                self.active = i;
            } else {
                self.open(path);
            }
        }
    }
    /// Read each listed worktree's changes and last commit.
    fn start_worktree_statuses(&mut self) {
        let Some(d) = &mut self.dialog else { return };
        let Some(s) = &mut d.worktrees else { return };
        let (Some(backend), Some(path)) = (
            self.backend.clone(),
            self.tabs.iter().find(|t| Some(t.id) == d.owner).map(|t| t.path.clone()),
        ) else {
            return;
        };
        let entries = s.entries.clone();
        match ReadTask::spawn(move |cancel| backend.worktree_statuses(&path, &entries, &cancel)) {
            Ok(task) => s.status_task = Some(task),
            Err(e) => d.error = Some(user_error(&e)),
        }
    }
    pub(crate) fn finish_worktree_dialog(&mut self, owner: Option<u64>, outcome: &Outcome) {
        let inherited = self.dialog.as_ref().filter(|d| d.owner == owner)
            .and_then(|d| d.worktrees.as_ref())
            .filter(|s| s.applying && s.creating && outcome.command_succeeded && !outcome.recovery_pending)
            .and_then(|s| s.preview.as_ref()).map(|p| p.path().to_owned());
        if let Some(path) = inherited { self.inherit_worktree_view(owner, &path); }
        let Some(d) = &mut self.dialog else { return };
        let Some(s) = &mut d.worktrees else { return };
        if !s.applying || owner != d.owner {
            return;
        }
        s.applying = false;
        if outcome.command_succeeded && !outcome.recovery_pending {
            if s.creating || s.maintaining.is_some() {
                s.selected = s.preview.as_ref().map(|p| p.path().to_owned());
            }
            s.creating = false;
            s.removing = false;
            s.maintaining = None;
            s.invalidate();
            d.first.clear();
            self.start_worktree_read();
        } else {
            s.invalidate();
            d.error = Some(trf!("{summary}\n{details}\nReturn to the list and refresh before retrying.", summary = outcome.summary, details = outcome.details.join("\n")));
        }
    }
    /// Worktrees page: list and details. Create and remove open the review dialog.
    pub(crate) fn worktree_page(&mut self, ui: &mut Ui) {
        let writable = self.can_write_repository();
        let default_target = self
            .tab()
            .and_then(|t| t.selected_commit.as_ref())
            .map_or_else(|| "HEAD".into(), ToString::to_string);
        let open_paths = self.open_worktree_paths();
        let here = self.tab().map(|t| {
            t.snapshot
                .as_ref()
                .and_then(|s| s.info.worktree.clone())
                .unwrap_or_else(|| t.path.clone())
        });
        let terminals_closing = !self.closing_terminals.is_empty();
        let Some(mut d) = self.dialog.take() else {
            return;
        };
        let s = d.worktrees.get_or_insert_with(Default::default);
        let reviewing = s.creating || s.removing || s.maintaining.is_some() || s.applying;
        let waiting = s.running() || s.applying;
        let mut f = Flags::default();
        // Under a review dialog the page is inert and publishes no controls.
        let mut inert = HashMap::new();
        let widgets = if reviewing {
            &mut inert
        } else {
            &mut self.frame_widgets
        };
        ui.add_enabled_ui(!reviewing, |ui| {
            let count = s.entries.iter().filter(|e| !e.bare).count();
            pages::header(
                ui,
                Icon::Worktree,
                crate::i18n::tr("Worktrees"),
                &trn!(count, "{n} working folder · each has its own branch and changes; the history is shared", "{n} working folders · each has its own branch and changes; the history is shared"),
                |ui| {
                    let r = pages::ghost(ui, !s.applying, crate::i18n::tr("Close"));
                    control(widgets, "dialog-cancel", &r, ui.clip_rect());
                    f.dismiss = r.clicked();
                    let r = pages::primary(ui, !waiting && writable, tr("New worktree"));
                    control(widgets, "dialog-worktree-create", &r, ui.clip_rect());
                    if r.clicked() {
                        s.creating = true;
                        s.target = default_target.clone();
                        s.suggested = None;
                        s.invalidate();
                        d.error = None;
                    }
                    let r = crate::chrome::icon_button(ui, Icon::Refresh, !waiting, false, 32.0)
                        .on_hover_text(crate::i18n::tr("Refresh"));
                    control(widgets, "dialog-preview-refresh", &r, ui.clip_rect());
                    f.refresh |= r.clicked();
                },
            );
            pages::divider(ui);
            if let Some(bare) = s.entries.iter().find(|e| e.bare) {
                pages::toolbar(ui, |ui| {
                    pages::callout(
                        ui,
                        BLUE(),
                        &trf!("{repository} is a bare repository: it has history and tags but no working folder. Add a worktree to check out a branch and edit files.", repository = folder_name(&bare.path)),
                    )
                });
            }
            if !reviewing && let Some(error) = &d.error {
                pages::toolbar(ui, |ui| pages::callout(ui, RED(), error));
            }
            let body = ui.available_rect_before_wrap();
            pages::split(ui, |ui, side| match side {
                pages::Side::List => {
                    let height = (ui.available_height() - 8.0).max(80.0);
                    let list = ScrollArea::vertical()
                        .id_salt("worktree-list")
                        .max_height(height)
                        .auto_shrink([false, true])
                        .show(ui, |ui| {
                            ui.spacing_mut().item_spacing.y = 2.0;
                            if s.entries.is_empty() && !waiting {
                                pages::empty(ui, tr("No worktrees loaded. Refresh to check the repository."));
                            }
                            if s.entries.is_empty() && waiting {
                                pages::loading(ui, tr("Reading worktrees"));
                            }
                            for (i, e) in s.entries.iter().enumerate() {
                                let status = s.statuses.iter().find(|st| st.path == e.path);
                                let r = worktree_row(
                                    ui,
                                    e,
                                    status,
                                    &pills(e, &open_paths, here.as_ref()),
                                    s.status_task.is_some(),
                                    s.selected.as_ref() == Some(&e.path),
                                    !waiting,
                                );
                                control(widgets, format!("dialog-worktree-row-{i}"), &r, ui.clip_rect());
                                if r.clicked() {
                                    s.selected = Some(e.path.clone());
                                    d.error = None;
                                }
                            }
                        });
                    pages::region(
                        widgets,
                        "dialog-worktree-list-scroll",
                        list.inner_rect.intersect(ui.clip_rect()),
                        !waiting,
                    );
                }
                pages::Side::Details => {
                    let Some(e) = s.selected_entry().cloned() else {
                        pages::empty(ui, tr("Select a worktree to see its folder, branch and changes."));
                        return;
                    };
                    let status = s.statuses.iter().find(|st| st.path == e.path).cloned();
                    ui.add(Label::new(RichText::new(folder_name(&e.path)).font(semibold(18.0))).wrap());
                    ui.horizontal_wrapped(|ui| {
                        for (text, color) in pills(&e, &open_paths, here.as_ref()) {
                            pages::pill(ui, &text, color);
                        }
                    });
                    ui.add_space(4.0);
                    pages::key_value(ui, crate::i18n::tr("Folder"), 92.0, |ui| {
                        ui.add(Label::new(crate::views::display_path(&e.path)).wrap());
                        ui.horizontal(|ui| {
                            let r = pages::ghost(ui, true, crate::i18n::tr("Copy path"));
                            control(widgets, "dialog-worktree-copy", &r, ui.clip_rect());
                            if r.clicked() {
                                ui.ctx().copy_text(e.path.display().to_string());
                            }
                            let r = pages::ghost(ui, !e.bare && e.prunable.is_none(), crate::i18n::tr("Show in Explorer"));
                            control(widgets, "dialog-worktree-reveal", &r, ui.clip_rect());
                            if r.clicked() {
                                f.reveal = Some(e.path.clone());
                            }
                        });
                    });
                    worktree_facts(ui, &e, status.as_ref(), s.status_task.is_some());
                    let open = open_paths.contains(&e.path);
                    let note = if e.bare {
                        Some((MUTED(), tr("Holds the repository itself. There is nothing to open or remove; add a worktree to work on files.").to_owned()))
                    } else if let Some(reason) = &e.prunable {
                        Some((AMBER(), trf!("The folder is gone ({reason}). If you moved it, find it; if you deleted it, forget it. No files are deleted either way.", reason = display(reason))))
                    } else if let Some(reason) = &e.locked {
                        Some((AMBER(), trf!("Locked: {reason}", reason = display(reason))))
                    } else if e.main {
                        Some((MUTED(), tr("The main worktree holds the repository, so it can't be removed or moved here.").to_owned()))
                    } else if open {
                        Some((MUTED(), tr("Open in Rocket. Close that tab before removing it; Open switches to it.").to_owned()))
                    } else {
                        None
                    };
                    if let Some((color, text)) = note {
                        pages::callout(ui, color, &text);
                    }
                    ui.add_space(6.0);
                    pages::divider(ui);
                    ui.add_space(4.0);
                    ui.horizontal_wrapped(|ui| {
                        if !e.bare && e.prunable.is_none() {
                            let label = if here.as_ref() == Some(&e.path) { tr("Open in this window") } else { tr("Open in Rocket") };
                            let r = pages::primary(ui, !waiting, label);
                            control(widgets, "dialog-worktree-open", &r, ui.clip_rect());
                            f.opening = r.clicked();
                        }
                        let can_maintain = !waiting && writable && !terminals_closing
                            && !e.main && !e.bare && e.locked.is_none() && !open;
                        for (kind, id, label) in [
                            (WorktreeMaintenance::Move, "move", tr("Move")),
                            (WorktreeMaintenance::Repair, "repair", tr("Find moved folder")),
                            (WorktreeMaintenance::Prune, "prune", tr("Forget it")),
                        ] {
                            let eligible = if kind == WorktreeMaintenance::Move { e.prunable.is_none() } else { e.prunable.is_some() };
                            if !eligible || e.main || e.bare {
                                continue;
                            }
                            let r = ui.add_enabled(can_maintain, Button::new(label));
                            control(widgets, format!("dialog-worktree-{id}"), &r, ui.clip_rect());
                            if r.clicked() {
                                s.maintaining = Some(kind);
                                s.invalidate();
                                d.first.clear();
                                d.error = None;
                                f.refresh = kind == WorktreeMaintenance::Prune;
                            }
                        }
                        let can_remove = !waiting
                            && writable
                            && !terminals_closing
                            && !e.main
                            && !e.bare
                            && e.locked.is_none()
                            && e.prunable.is_none()
                            && !open;
                        if !e.main && !e.bare && e.prunable.is_none() {
                            let r = pages::danger_ghost(ui, can_remove, tr("Remove"));
                            control(widgets, "dialog-worktree-remove", &r, ui.clip_rect());
                            if r.clicked() {
                                s.removing = true;
                                f.refresh = true;
                            }
                        }
                    });
                }
            });
            pages::details_scroll(ui, widgets, !s.applying);
            pages::region(widgets, "dialog-worktrees-scroll", body.intersect(ui.clip_rect()), !s.applying);
        });
        self.finish_worktree_frame(d, f);
    }

    /// Create/remove review dialog over the Worktrees page. Also drops a page whose
    /// repository is no longer active.
    pub(crate) fn worktree_dialog(&mut self, ctx: &Context) {
        let writable = self.can_write_repository();
        let open_paths = self.open_worktree_paths();
        let terminals_closing = !self.closing_terminals.is_empty();
        let remote_branches = self.tab().map(remote_branches).unwrap_or_default();
        let remote_names: Vec<String> = remote_branches.iter().map(|(n, _)| n.clone()).collect();
        let local_branches = self.tab().map(free_local_branches).unwrap_or_default();
        let all_branches: Vec<Branch> = self
            .tab()
            .and_then(|t| t.snapshot.as_ref())
            .map(|s| s.branches.clone())
            .unwrap_or_default();
        let current = head_name(self.tab().and_then(|t| t.snapshot.as_ref()).and_then(|s| s.head.as_ref()));
        let repository = self.tab().map(|t| t.path.clone());
        let Some(mut d) = self.dialog.take() else {
            return;
        };
        let branch_space = d.branch_space;
        let s = d.worktrees.get_or_insert_with(Default::default);
        if d.owner != self.tab().map(|t| t.id) && !s.applying {
            return;
        }
        if !s.creating && !s.removing && s.maintaining.is_none() && !s.applying {
            self.dialog = Some(d);
            return;
        }
        let waiting = s.running() || s.applying;
        let mut f = Flags::default();
        let creating = s.creating;
        let maintaining = s.maintaining;
        let selected = s.selected_entry().cloned();
        let selected_status = selected
            .as_ref()
            .and_then(|e| s.statuses.iter().find(|st| st.path == e.path))
            .cloned();
        let remove_title = trf!("Remove {worktree}?", worktree = selected.as_ref().map_or_else(|| tr("this worktree").to_owned(), |e| folder_name(&e.path)));
        let head = pages::ReviewHeader {
            image: Icon::Worktree,
            title: if let Some(kind) = maintaining {
                maintenance_title(kind)
            } else if creating {
                tr("New worktree")
            } else {
                &remove_title
            },
            subtitle: if maintaining.is_some() {
                tr("Only this worktree is affected.")
            } else if creating {
                tr("Another working folder for this repository, with its own branch.")
            } else {
                tr("Deletes the folder. The branch and its commits stay.")
            },
            danger: !creating
                && (maintaining.is_none() || maintaining == Some(WorktreeMaintenance::Prune)),
        };
        let running = pages::Running::of(self.write.as_ref());
        // Why the create review can't start yet, in words.
        let create_problem = |s: &DialogState, folder: &str| -> Option<String> {
            let branch = s.branch.trim();
            if branch.is_empty() {
                return Some(crate::i18n::tr("Name the branch").into());
            }
            if s.existing {
                if !local_branches.iter().any(|b| b == branch) {
                    return Some(tr("Choose a branch that isn't checked out anywhere").into());
                }
            } else if let Some(problem) = crate::small_dialogs::branch_name_problem(branch, &all_branches, None) {
                return Some(problem);
            }
            if s.remote && s.upstream.trim().is_empty() {
                return Some(tr("Choose the remote branch to track").into());
            }
            if folder.trim().is_empty() {
                return Some(tr("Choose a folder").into());
            }
            None
        };
        let response = pages::review(
            ctx,
            "rocket-worktrees",
            head,
            &mut self.frame_widgets,
            "dialog-worktrees-scroll",
            !s.applying,
            |ui, part, widgets| match part {
                pages::Part::Body => {
                    if let Some(kind) = maintaining {
                        if let Some(entry) = &selected {
                            worktree_card(ui, tr("Worktree"), entry, selected_status.as_ref());
                        }
                        if kind != WorktreeMaintenance::Prune {
                            ui.label(RichText::new(if kind == WorktreeMaintenance::Move {
                                tr("New place for the folder · must not exist yet, same drive")
                            } else {
                                tr("Where the folder is now")
                            }).size(12.0).color(MUTED()));
                            let r = ui.add_enabled(
                                !s.applying,
                                TextEdit::singleline(&mut d.first)
                                    .desired_width(f32::INFINITY)
                                    .min_size(vec2(0.0, 30.0))
                                    .vertical_align(Align::Center),
                            );
                            control(widgets, "dialog-worktree-path", &r, ui.clip_rect());
                            if r.changed() {
                                s.invalidate();
                                d.error = None;
                            }
                        }
                        pages::callout(
                            ui,
                            if kind == WorktreeMaintenance::Prune { AMBER() } else { BLUE() },
                            match kind {
                                WorktreeMaintenance::Move => tr("The whole folder moves. Close editors using it first."),
                                WorktreeMaintenance::Repair => tr("Reconnects this worktree to its moved folder."),
                                WorktreeMaintenance::Prune => {
                                    tr("Only Git's record of it goes. If the folder was moved rather than deleted, use Find moved folder instead.")
                                }
                            },
                        );
                        if let Some(p) = &s.preview {
                            pages::key_value(ui, tr("Reviewed folder"), 112.0, |ui| {
                                ui.add(Label::new(crate::views::display_path(p.path())).wrap());
                            });
                        }
                    } else if creating {
                        let mut edited = false;
                        ui.label(RichText::new(crate::i18n::tr("Branch")).size(12.0).color(MUTED()));
                        let kinds = [
                            (false, false, crate::i18n::tr("New branch"), tr("Start one from any commit"), "new"),
                            (true, false, tr("Existing branch"), tr("One not checked out anywhere"), "existing"),
                            (false, true, tr("Remote branch"), tr("Track a branch on a remote"), "remote"),
                        ];
                        let cards: Vec<_> = kinds
                            .iter()
                            .map(|&(existing, remote, name, about, _)| (s.existing == existing && s.remote == remote, name, about))
                            .collect();
                        for (r, &(existing, remote, _, _, id)) in pages::option_cards(ui, !s.applying, &cards).iter().zip(&kinds) {
                            control(widgets, format!("dialog-worktree-{id}"), r, ui.clip_rect());
                            if r.clicked() && !(s.existing == existing && s.remote == remote) {
                                s.existing = existing;
                                s.remote = remote;
                                edited = true;
                            }
                        }
                        ui.add_space(2.0);
                        if s.remote {
                            ui.label(RichText::new(tr("Remote branch to track")).size(12.0).color(MUTED()));
                            ui.horizontal(|ui| {
                                let width = (ui.available_width() - 150.0).max(160.0);
                                let r = ui.add_enabled(
                                    !s.applying,
                                    TextEdit::singleline(&mut s.upstream)
                                        .hint_text(RichText::new("origin/main").color(FAINT()))
                                        .desired_width(width)
                                        .min_size(vec2(0.0, 30.0))
                                        .vertical_align(Align::Center),
                                );
                                control(widgets, "dialog-worktree-upstream", &r, ui.clip_rect());
                                edited |= r.changed();
                                if let Some(name) = crate::branch_picker::pick(
                                    ui,
                                    widgets,
                                    "dialog-worktree-upstream-choice",
                                    crate::i18n::tr("Remote branches"),
                                    &remote_names,
                                    !s.applying,
                                ) {
                                    // Suggest the same name locally unless one was typed.
                                    if s.branch.trim().is_empty()
                                        || remote_branches.iter().any(|(_, l)| *l == s.branch)
                                    {
                                        s.branch = remote_branches
                                            .iter()
                                            .find(|(n, _)| *n == name)
                                            .map(|(_, l)| l.clone())
                                            .unwrap_or_default();
                                    }
                                    s.upstream = name;
                                    edited = true;
                                }
                            });
                        }
                        ui.label(
                            RichText::new(if s.remote {
                                tr("Local name")
                            } else if s.existing {
                                crate::i18n::tr("Branch")
                            } else {
                                tr("New branch name")
                            })
                            .size(12.0)
                            .color(MUTED()),
                        );
                        ui.horizontal(|ui| {
                            let width = if s.existing { (ui.available_width() - 150.0).max(160.0) } else { ui.available_width() };
                            let r = ui.add_enabled(
                                !s.applying,
                                TextEdit::singleline(&mut s.branch)
                                    .hint_text(RichText::new("feature/my-change").color(FAINT()))
                                    .desired_width(width)
                                    .min_size(vec2(0.0, 30.0))
                                    .vertical_align(Align::Center),
                            );
                            control(widgets, "dialog-worktree-branch", &r, ui.clip_rect());
                            edited |= r.changed();
                            if r.changed() && !s.existing {
                                branch_space.apply(&mut s.branch);
                            }
                            if s.existing
                                && let Some(name) = crate::branch_picker::pick(
                                    ui,
                                    widgets,
                                    "dialog-worktree-branch-choice",
                                    tr("Free branches"),
                                    &local_branches,
                                    !s.applying,
                                )
                            {
                                s.branch = name;
                                edited = true;
                            }
                        });
                        let branch = s.branch.trim().to_owned();
                        if !branch.is_empty() {
                            let problem = if s.existing {
                                (!local_branches.contains(&branch))
                                    .then(|| tr("Choose a branch that isn't checked out anywhere.").to_owned())
                            } else {
                                crate::small_dialogs::branch_name_problem(&branch, &all_branches, None)
                            };
                            if let Some(problem) = problem {
                                crate::tags::check(ui, false, &problem);
                            }
                        }
                        if !s.existing && !s.remote {
                            ui.label(RichText::new(crate::i18n::tr("Start from")).size(12.0).color(MUTED()));
                            let r = ui.add_enabled(
                                !s.applying,
                                TextEdit::singleline(&mut s.target)
                                    .hint_text(RichText::new(tr("HEAD, a branch or a commit ID")).color(FAINT()))
                                    .desired_width(f32::INFINITY)
                                    .min_size(vec2(0.0, 30.0))
                                    .vertical_align(Align::Center),
                            );
                            control(widgets, "dialog-worktree-target", &r, ui.clip_rect());
                            edited |= r.changed();
                        }
                        // The folder follows the branch name until it is edited.
                        if let Some(root) = &repository
                            && !branch.is_empty()
                            && (d.first.trim().is_empty() || s.suggested.as_deref() == Some(d.first.as_str()))
                        {
                            let leaf = branch.rsplit('/').next().unwrap_or(&branch);
                            let name = format!("{}-{leaf}", folder_name(root));
                            if let Some(parent) = root.parent() {
                                // Without the \\?\ prefix of the repository's own path.
                                let suggestion = crate::views::display_path(&parent.join(name));
                                if d.first != suggestion {
                                    d.first = suggestion.clone();
                                    edited = true;
                                }
                                s.suggested = Some(suggestion);
                            }
                        }
                        ui.label(RichText::new(crate::i18n::tr("Folder")).size(12.0).color(MUTED()));
                        ui.horizontal(|ui| {
                            let width = (ui.available_width() - 84.0).max(160.0);
                            let r = ui.add_enabled(
                                !s.applying,
                                TextEdit::singleline(&mut d.first)
                                    .desired_width(width)
                                    .min_size(vec2(0.0, 30.0))
                                    .vertical_align(Align::Center),
                            );
                            control(widgets, "dialog-worktree-path", &r, ui.clip_rect());
                            if r.changed() {
                                s.suggested = None;
                                edited = true;
                            }
                            let r = ui.add_enabled(!s.applying, Button::new(crate::i18n::tr("Browse")).min_size(vec2(76.0, 30.0)));
                            control(widgets, "dialog-worktree-browse", &r, ui.clip_rect());
                            f.browse = r.clicked();
                        });
                        pages::note(ui, tr("A new folder; Rocket suggests one next to the repository."));
                        if edited {
                            s.invalidate();
                            d.error = None;
                        }
                        if let Some(p) = &s.preview {
                            let branch = p.checkout().map_or(branch.clone(), |c| c.branch().to_owned());
                            let folder = folder_name(p.path());
                            let text = match p.checkout().and_then(WorktreeCheckout::upstream) {
                                Some(upstream) => trf!("A new folder {folder} with {branch} checked out, tracking {upstream}. This window stays on {current}.", folder, branch, upstream, current),
                                None => trf!("A new folder {folder} with {branch} checked out. This window stays on {current}.", folder, branch, current),
                            };
                            pages::callout(ui, BLUE(), &text);
                        }
                    } else if let Some(e) = &selected {
                        let changes = selected_status.as_ref().map(|st| st.changes);
                        crate::tags::card(ui, tr("What goes"), |ui| {
                            pages::key_value(ui, crate::i18n::tr("Folder"), 104.0, |ui| {
                                ui.add(Label::new(crate::views::display_path(&e.path)).wrap());
                            });
                            pages::key_value(ui, crate::i18n::tr("Changes"), 104.0, |ui| match changes {
                                Some(0) => {
                                    ui.label(RichText::new(tr("None · nothing to lose")).color(GREEN()));
                                }
                                Some(n) => {
                                    ui.label(RichText::new(changed_files(n)).color(AMBER()));
                                }
                                None => {
                                    ui.label(RichText::new(tr("Checked when you remove it")).color(MUTED()));
                                }
                            });
                        });
                        crate::tags::card(ui, tr("What stays"), |ui| {
                            pages::key_value(ui, crate::i18n::tr("Branch"), 104.0, |ui| {
                                ui.label(branch_label(e));
                            });
                            last_commit(ui, e, selected_status.as_ref());
                        });
                        // A failed review's error, below, says why instead.
                        let (color, note) = if changes.is_some_and(|n| n > 0) {
                            (AMBER(), tr("Commit or stash its changes there first. Rocket doesn't remove folders with work in them."))
                        } else {
                            (BLUE(), tr("Only the folder goes. You can make the worktree again later from the same branch."))
                        };
                        if d.error.is_none() {
                            pages::callout(ui, color, note);
                        }
                    }
                    // A running worktree operation shows its status and Stop in the footer.
                    if waiting && !s.applying {
                        pages::waiting(ui, tr("Checking the worktree"));
                    }
                    if let Some(error) = &d.error {
                        pages::callout(ui, RED(), error);
                    }
                }
                pages::Part::Footer if s.applying => {
                    f.cancel = pages::running_footer(ui, widgets, &running, &[]);
                }
                pages::Part::Footer => {
                    let eligible = !(s.removing || maintaining.is_some())
                        || s.preview.as_ref().is_some_and(|p| {
                            !open_paths.iter().any(|path| {
                                path == p.path() || p.entry().is_some_and(|e| path == &e.path)
                            }) && !terminals_closing
                        });
                    let ready = !waiting
                        && writable
                        && eligible
                        && s.preview.is_some()
                        && d.error.is_none();
                    let r = if let Some(kind) = maintaining {
                        if kind == WorktreeMaintenance::Prune {
                            pages::danger(ui, ready, maintenance_title(kind))
                        } else {
                            pages::primary(ui, ready, maintenance_title(kind))
                        }
                    } else if creating {
                        pages::primary(ui, ready, crate::i18n::tr("Create worktree"))
                    } else {
                        pages::danger(ui, ready, crate::i18n::tr("Remove worktree"))
                    };
                    control(widgets, "dialog-submit", &r, ui.clip_rect());
                    f.submit = r.clicked();
                    let problem = if creating { create_problem(s, &d.first) } else { None };
                    let can_review = !waiting && problem.is_none();
                    let r = if creating && s.preview.is_none() && d.error.is_none() {
                        pages::primary(ui, can_review, crate::i18n::tr("Review"))
                    } else {
                        ui.add_enabled(can_review, Button::new(tr("Review again")))
                    };
                    control(widgets, "dialog-preview-refresh", &r, ui.clip_rect());
                    f.refresh |= r.clicked();
                    let status = if waiting {
                        tr("Reviewing").to_owned()
                    } else if d.error.is_some() {
                        tr("Fix the problem above, then review again").to_owned()
                    } else if let Some(problem) = problem {
                        problem
                    } else if s.preview.is_some() {
                        tr("Reviewed · nothing has changed yet").to_owned()
                    } else if creating {
                        tr("Review to see what will be created").to_owned()
                    } else {
                        String::new()
                    };
                    pages::footer_status(ui, false, &status);
                }
            },
        );
        // The × returns to the Worktrees page; automation's old Cancel id is that ×.
        if let Some(close) = self.frame_widgets.get("dialog-close").copied() {
            self.frame_widgets.insert("dialog-worktree-back".into(), close);
        }
        f.back |= response.should_close() && !s.applying;
        self.finish_worktree_frame(d, f);
    }

    fn open_worktree_paths(&self) -> Vec<PathBuf> {
        self.tabs
            .iter()
            .map(|t| {
                t.snapshot
                    .as_ref()
                    .and_then(|s| s.info.worktree.clone())
                    .unwrap_or_else(|| t.path.clone())
            })
            .collect()
    }

    /// Apply one frame's page or review choices.
    fn finish_worktree_frame(&mut self, mut d: Dialog, mut f: Flags) {
        let s = d.worktrees.get_or_insert_with(Default::default);
        if f.back {
            s.creating = false;
            s.removing = false;
            s.maintaining = None;
            s.invalidate();
            d.error = None;
            f.refresh = true;
        }
        if f.cancel {
            self.cancel_write();
        }
        if f.opening
            && let (Some(backend), Some(entry), Some(path)) = (
                self.backend.clone(),
                s.selected_entry().cloned(),
                self.tabs
                    .iter()
                    .find(|t| Some(t.id) == d.owner)
                    .map(|t| t.path.clone()),
            )
        {
            d.error = None;
            match ReadTask::spawn(move |cancel| backend.worktree_open(&path, &entry, &cancel)) {
                Ok(t) => s.open_task = Some(t),
                Err(e) => d.error = Some(user_error(&e)),
            }
        }
        if f.submit
            && let (Some(owner), Some(p)) = (d.owner, s.preview.clone())
        {
            s.applying = true;
            self.execute(owner, Action::Worktree(p));
            if self.write.is_none() {
                s.applying = false;
                d.error =
                    Some(tr("Could not start. Wait for repository refresh, then review again.").into());
            }
        }
        let folder = d.first.clone();
        if !f.dismiss {
            self.dialog = Some(d);
        }
        if f.refresh {
            self.start_worktree_read();
        }
        if let Some(path) = f.reveal {
            self.dispatch(Intent::ShowFolder(path));
        }
        if f.browse {
            self.browse(crate::folder_browser::PickTarget::WorktreeFolder, &folder);
        }
    }
}

#[derive(Default)]
struct Flags {
    dismiss: bool,
    refresh: bool,
    submit: bool,
    opening: bool,
    back: bool,
    cancel: bool,
    /// Show this folder in Explorer.
    reveal: Option<PathBuf>,
    /// Choose the new worktree's folder.
    browse: bool,
}

/// Remote branches a new tracking branch can follow, such as `origin/main`, each with
/// the local name it suggests (`main`). Symbolic names such as `origin/HEAD` are left out.
fn remote_branches(tab: &Tab) -> Vec<(String, String)> {
    let Some(snapshot) = &tab.snapshot else {
        return Vec::new();
    };
    snapshot
        .branches
        .iter()
        .filter(|b| b.kind == BranchKind::Remote && b.symbolic_target.is_none())
        .filter_map(|b| {
            let name = display(b.reference.strip_prefix(b"refs/remotes/")?);
            let local = snapshot
                .remotes
                .iter()
                .find_map(|r| name.strip_prefix(&format!("{}/", display(&r.name))))
                .unwrap_or(&name)
                .to_owned();
            Some((name, local))
        })
        .collect()
}

/// Local branches not checked out in any worktree, which a new worktree can use.
fn free_local_branches(tab: &Tab) -> Vec<String> {
    let Some(snapshot) = &tab.snapshot else {
        return Vec::new();
    };
    snapshot
        .branches
        .iter()
        .filter(|b| {
            b.kind == BranchKind::Local
                && b.symbolic_target.is_none()
                && b.checked_out_at.is_none()
                && !b.is_head
        })
        .filter_map(|b| b.reference.strip_prefix(b"refs/heads/").map(display))
        .collect()
}

fn branch_label(e: &WorktreeEntry) -> String {
    e.branch
        .as_ref()
        .map(|b| display(b.strip_prefix(b"refs/heads/").unwrap_or(b)))
        .unwrap_or_else(|| {
            if e.bare {
                tr("Bare repository").into()
            } else {
                crate::i18n::tr("Detached HEAD").into()
            }
        })
}
fn maintenance_title(kind: WorktreeMaintenance) -> &'static str {
    match kind {
        WorktreeMaintenance::Move => tr("Move worktree"),
        WorktreeMaintenance::Repair => tr("Repair worktree"),
        WorktreeMaintenance::Prune => tr("Prune registration"),
    }
}

/// A path's last part, the name the worktree is known by.
fn folder_name(path: &Path) -> String {
    path.file_name()
        .map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned())
}

fn changed_files(n: usize) -> String {
    trn!(n, "{n} changed file", "{n} changed files")
}

/// A worktree's pills: what kind it is, whether it is open, and what needs attention.
fn pills(e: &WorktreeEntry, open: &[PathBuf], here: Option<&PathBuf>) -> Vec<(String, Color32)> {
    let mut pills = vec![if e.bare {
        (tr("Bare").to_owned(), pages::NEUTRAL())
    } else if e.main {
        (tr("Main").to_owned(), pages::INFO())
    } else {
        (tr("Linked").to_owned(), pages::NEUTRAL())
    }];
    if here == Some(&e.path) {
        pills.push((tr("This window").into(), pages::INFO()));
    } else if open.contains(&e.path) {
        pills.push((crate::i18n::tr("Open").into(), pages::OK()));
    }
    if e.locked.is_some() {
        pills.push((crate::i18n::tr("Locked").into(), pages::WARN()));
    }
    if e.prunable.is_some() {
        pills.push((tr("Folder missing").into(), pages::WARN()));
    }
    pills
}

/// What a worktree's changes look like in words, and their color.
fn changes_text(e: &WorktreeEntry, status: Option<&WorktreeStatus>, checking: bool) -> (String, Color32) {
    if e.bare {
        return (tr("no working folder").into(), MUTED());
    }
    if e.prunable.is_some() {
        return (tr("folder missing").into(), AMBER());
    }
    match status {
        Some(s) if s.changes == 0 => (tr("Clean").into(), GREEN()),
        Some(s) => (changed_files(s.changes), AMBER()),
        None if checking => (tr("Checking").into(), MUTED()),
        None => (String::new(), MUTED()),
    }
}

/// One worktree: its folder and pills, its branch and changes, then its path; the
/// last commit on the right.
fn worktree_row(
    ui: &mut Ui,
    e: &WorktreeEntry,
    status: Option<&WorktreeStatus>,
    pills: &[(String, Color32)],
    checking: bool,
    selected: bool,
    enabled: bool,
) -> Response {
    let (rect, response) = ui
        .add_enabled_ui(enabled, |ui| {
            ui.allocate_exact_size(vec2(ui.available_width(), 62.0), Sense::click())
        })
        .inner;
    let painter = ui.painter();
    if selected {
        painter.rect_filled(rect, 8.0, SELECTED());
        painter.rect_filled(
            Rect::from_min_size(rect.min + vec2(0.0, 12.0), vec2(3.0, rect.height() - 24.0)),
            1.5,
            BLUE(),
        );
    } else if enabled && (response.hovered() || response.has_focus()) {
        painter.rect_filled(rect, 8.0, HOVER());
    }
    let tile = Rect::from_min_size(pos2(rect.left() + 12.0, rect.center().y - 16.0), vec2(32.0, 32.0));
    painter.rect_filled(tile, 7.0, CARD());
    icon(
        painter,
        tile.shrink(8.0),
        Icon::Folder,
        if selected { BLUE() } else if e.prunable.is_some() { AMBER() } else { HINT() },
    );
    let left = tile.right() + 12.0;
    let right = rect.right() - 12.0;
    // The last commit, right.
    let when = match (&e.head, status.and_then(|s| s.last.as_ref())) {
        (Some(head), Some((_, time))) => format!("{} · {}", &head.as_str()[..7], crate::tags::age(*time)),
        (Some(head), None) => head.as_str()[..7].to_owned(),
        (None, _) => String::new(),
    };
    let when = painter.layout_no_wrap(when, FontId::proportional(12.0), HINT());
    let when_width = when.size().x;
    painter.galley(pos2(right - when_width, rect.top() + 12.0), when, HINT());
    let width = (right - when_width - 12.0 - left).max(60.0);
    // Folder name and pills.
    let mut x = left;
    let line = rect.top() + 14.0;
    let mut job = text::LayoutJob::simple_singleline(folder_name(&e.path), semibold(13.0), TEXT());
    job.wrap = text::TextWrapping::truncate_at_width((width * 0.6).max(60.0));
    let name = painter.layout_job(job);
    let name_width = name.size().x;
    painter.galley(pos2(x, line - name.size().y / 2.0), name, TEXT());
    x += name_width + 8.0;
    for (text, color) in pills {
        let galley = painter.layout_no_wrap(text.clone(), FontId::proportional(11.5), *color);
        let pill = Rect::from_min_size(pos2(x, line - 8.5), vec2(galley.size().x + 14.0, 17.0));
        if pill.right() > left + width {
            break;
        }
        painter.rect_filled(pill, 8.5, color.gamma_multiply(0.14));
        painter.galley(pos2(pill.left() + 7.0, pill.center().y - galley.size().y / 2.0), galley, *color);
        x = pill.right() + 5.0;
    }
    // Branch and changes.
    let (changes, color) = changes_text(e, status, checking);
    let mut job = text::LayoutJob::default();
    job.append(&branch_label(e), 0.0, TextFormat::simple(FontId::proportional(12.5), MUTED()));
    if !changes.is_empty() {
        job.append(" · ", 0.0, TextFormat::simple(FontId::proportional(12.5), FAINT()));
        job.append(&changes, 0.0, TextFormat::simple(FontId::proportional(12.5), color));
    }
    job.wrap = text::TextWrapping::truncate_at_width(width);
    let branch = painter.layout_job(job);
    painter.galley(pos2(left, rect.center().y - branch.size().y / 2.0 + 1.0), branch, MUTED());
    // The path.
    let mut job = text::LayoutJob::simple_singleline(
        crate::views::display_path(&e.path),
        FontId::proportional(12.0),
        FAINT(),
    );
    job.wrap = text::TextWrapping::truncate_at_width(right - left);
    let path = painter.layout_job(job);
    painter.galley(pos2(left, rect.bottom() - 11.0 - path.size().y / 2.0), path, FAINT());
    response.widget_info(|| WidgetInfo::selected(WidgetType::Button, enabled, selected, folder_name(&e.path)));
    response
}

/// The worktree's last commit as a card row.
fn last_commit(ui: &mut Ui, e: &WorktreeEntry, status: Option<&WorktreeStatus>) {
    pages::key_value(ui, tr("Last commit"), 92.0, |ui| match (&e.head, status.and_then(|s| s.last.as_ref())) {
        (Some(head), Some((subject, time))) => {
            let mut job = text::LayoutJob::default();
            job.append(&head.as_str()[..7], 0.0, TextFormat::simple(FontId::monospace(12.5), TEXT()));
            let subject: String = subject.chars().take(240).collect();
            job.append(&format!(" {subject}"), 0.0, TextFormat::simple(FontId::proportional(13.0), TEXT()));
            job.wrap = text::TextWrapping::truncate_at_width(ui.available_width());
            ui.label(job);
            ui.label(RichText::new(crate::tags::age(*time)).size(12.0).color(HINT()));
        }
        (Some(head), None) => {
            ui.label(RichText::new(&head.as_str()[..7]).monospace().size(12.5));
        }
        (None, _) => {
            ui.label(RichText::new(tr("None yet")).color(MUTED()));
        }
    });
}

/// Branch, last commit and changes, as card rows.
fn worktree_facts(ui: &mut Ui, e: &WorktreeEntry, status: Option<&WorktreeStatus>, checking: bool) {
    pages::key_value(ui, crate::i18n::tr("Branch"), 92.0, |ui| {
        ui.add(Label::new(branch_label(e)).wrap());
    });
    // Git lists no checked-out commit for the bare entry, though it has history.
    if !e.bare {
        last_commit(ui, e, status);
    }
    let (changes, color) = changes_text(e, status, checking);
    if !changes.is_empty() {
        pages::key_value(ui, crate::i18n::tr("Changes"), 92.0, |ui| {
            let mut chars = changes.chars();
            let text: String = match chars.next() {
                Some(first) => first.to_uppercase().chain(chars).collect(),
                None => String::new(),
            };
            ui.label(RichText::new(text).color(color));
        });
    }
}

/// A worktree's folder and facts in one card, for the maintenance reviews.
fn worktree_card(ui: &mut Ui, title: &str, e: &WorktreeEntry, status: Option<&WorktreeStatus>) {
    crate::tags::card(ui, title, |ui| {
        pages::key_value(ui, crate::i18n::tr("Folder"), 92.0, |ui| {
            ui.add(Label::new(crate::views::display_path(&e.path)).wrap());
        });
        worktree_facts(ui, e, status, false);
        if let Some(reason) = &e.locked {
            pages::callout(ui, AMBER(), &trf!("Locked: {reason}", reason = display(reason)));
        }
        if let Some(reason) = &e.prunable {
            pages::callout(ui, AMBER(), &trf!("Folder missing: {reason}", reason = display(reason)));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_common as common;
    use std::{
        thread,
        time::{Duration, Instant},
    };

    #[test]
    fn repository_switch_dismisses_worktree_preview_without_creating() {
        common::isolated(
            "ROCKET_WORKTREE_UI_CASE",
            "worktrees::tests::repository_switch_dismisses_worktree_preview_without_creating",
            || {
                let first = common::Fixture::new();
                first.write("file", "first");
                first.commit("first");
                let second = common::Fixture::new();
                second.write("file", "second");
                second.commit("second");
                let ctx = Context::default();
                let mut app = Rocket::with_log_directory(
                    &ctx,
                    vec![first.repository.clone(), second.repository.clone()],
                    first.root.join("logs"),
                );
                let wait = |app: &mut Rocket| {
                    let deadline = Instant::now() + Duration::from_secs(40);
                    while app.busy() {
                        app.poll(&ctx);
                        assert!(Instant::now() < deadline);
                        thread::sleep(Duration::from_millis(10));
                    }
                };
                wait(&mut app);
                app.auto_refresh = false;
                app.active = 0;
                app.dispatch(Intent::Dialog(DialogKind::Worktrees));
                wait(&mut app);
                let destination = first.root.canonicalize().unwrap().join("preview-only");
                let dialog = app.dialog.as_mut().unwrap();
                dialog.first = destination.to_string_lossy().into_owned();
                let state = dialog.worktrees.as_mut().unwrap();
                state.creating = true;
                state.branch = "preview-only".into();
                state.target = "HEAD".into();
                app.start_worktree_read();
                wait(&mut app);
                assert!(
                    app.dialog
                        .as_ref()
                        .unwrap()
                        .worktrees
                        .as_ref()
                        .unwrap()
                        .preview
                        .is_some()
                );
                app.active = 1;
                let mut output = ctx.run_ui(Default::default(), |_| app.worktree_dialog(&ctx));
                output.textures_delta.clear();
                assert!(app.dialog.is_none());
                assert!(app.write.is_none());
                assert!(!destination.exists());
                for f in [&first, &second] {
                    assert_eq!(
                        f.repo().worktrees(&CancellationToken::new()).unwrap().len(),
                        1
                    );
                }
            },
        );
    }

    /// Each row says whether its folder has changes, and its last commit.
    #[test]
    fn rows_show_each_worktrees_changes_and_last_commit() {
        use crate::batch::flow_tests::{open, wait};
        common::isolated(
            "ROCKET_WORKTREE_UI_CASE",
            "worktrees::tests::rows_show_each_worktrees_changes_and_last_commit",
            || {
                let f = common::Fixture::new();
                f.write("file", "base");
                f.commit("PRIVATE_base");
                // Git refuses Windows' \\?\ spelling, so the ordinary path goes to it.
                let linked = f.root.join("linked 文");
                f.run(&["worktree", "add", "-q", "-b", "side", linked.to_str().unwrap()]);
                std::fs::write(linked.join("file"), "changed").unwrap();
                std::fs::write(linked.join("new.txt"), "new").unwrap();
                let ctx = Context::default();
                let mut app = open(&ctx, &f);
                app.dispatch(Intent::Dialog(DialogKind::Worktrees));
                wait(&ctx, &mut app);
                let s = app.dialog.as_ref().unwrap().worktrees.as_ref().unwrap();
                let same = |a: &std::path::Path, b: &std::path::Path| {
                    a.canonicalize().ok().is_some_and(|a| Some(a) == b.canonicalize().ok())
                };
                let changes = |path: &std::path::Path| {
                    s.statuses.iter().find(|st| same(&st.path, path)).map(|st| st.changes)
                };
                assert_eq!(s.statuses.len(), 2);
                assert_eq!(changes(&f.repository), Some(0));
                assert_eq!(changes(&linked), Some(2));
                assert!(s
                    .statuses
                    .iter()
                    .all(|st| st.last.as_ref().is_some_and(|(subject, _)| subject == "PRIVATE_base")));
            },
        );
    }

    #[test]
    fn a_remote_branch_is_checked_out_as_a_new_tracking_branch() {
        use crate::batch::flow_tests::{click, open, wait};
        common::isolated(
            "ROCKET_WORKTREE_UI_CASE",
            "worktrees::tests::a_remote_branch_is_checked_out_as_a_new_tracking_branch",
            || {
                let f = common::Fixture::new();
                f.write("file", "base");
                f.commit("base");
                // A remote branch without any network: the ref and its remote's refspec.
                f.run(&[
                    "config",
                    "remote.origin.url",
                    "https://example.invalid/r.git",
                ]);
                f.run(&[
                    "config",
                    "remote.origin.fetch",
                    "+refs/heads/*:refs/remotes/origin/*",
                ]);
                f.run(&["update-ref", "refs/remotes/origin/topic/remote", "HEAD"]);
                f.run(&[
                    "symbolic-ref",
                    "refs/remotes/origin/HEAD",
                    "refs/remotes/origin/topic/remote",
                ]);
                let ctx = Context::default();
                let mut app = open(&ctx, &f);
                // The list offers each remote branch once, with the local name it suggests.
                assert_eq!(
                    remote_branches(app.tab().unwrap()),
                    [("origin/topic/remote".to_owned(), "topic/remote".to_owned())]
                );
                app.dispatch(Intent::Dialog(DialogKind::Worktrees));
                wait(&ctx, &mut app);
                click(&ctx, &mut app, "dialog-worktree-create");
                click(&ctx, &mut app, "dialog-worktree-remote");
                let destination = f.root.canonicalize().unwrap().join("tracking 文");
                let dialog = app.dialog.as_mut().unwrap();
                dialog.first = destination.to_string_lossy().into_owned();
                let state = dialog.worktrees.as_mut().unwrap();
                assert!(state.remote && !state.existing);
                state.upstream = "origin/topic/remote".into();
                state.branch = "topic/remote".into();
                app.start_worktree_read();
                wait(&ctx, &mut app);
                let dialog = app.dialog.as_ref().unwrap();
                let preview = dialog.worktrees.as_ref().unwrap().preview.clone();
                let preview = preview.unwrap_or_else(|| panic!("{:?}", dialog.error));
                assert_eq!(
                    preview.checkout().and_then(WorktreeCheckout::upstream),
                    Some("origin/topic/remote")
                );
                click(&ctx, &mut app, "dialog-submit");
                wait(&ctx, &mut app);
                assert!(destination.join("file").exists());
                assert_eq!(
                    String::from_utf8(f.run(&[
                        "rev-parse",
                        "--abbrev-ref",
                        "topic/remote@{upstream}"
                    ]))
                    .unwrap()
                    .trim(),
                    "origin/topic/remote"
                );
            },
        );
    }
}
