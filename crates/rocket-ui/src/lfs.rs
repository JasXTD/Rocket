//! Owner-bound, cancellable LFS inspection and reviewed writes.
pub mod locks;
use crate::{
    app::*,
    backend::{Action, Outcome, user_error},
    pages::{self, control},
    style::*,
};
use eframe::egui::*;
use rocket_git::*;
use std::{collections::HashMap, sync::Arc};
#[derive(Default)]
pub struct DialogState {
    pub locks_tab: bool,
    pub locks: locks::DialogState,
    pub status: Option<LfsStatus>,
    pub selected: Option<RepoPath>,
    pub remote: String,
    pub action: Option<LfsAction>,
    pub preview: Option<Arc<LfsPreview>>,
    pub list_task: Option<ReadTask<LfsStatus>>,
    pub task: Option<ReadTask<LfsPreview>>,
    pub applying: bool,
    pub notice: Option<String>,
    pub tracking: bool,
    pub pattern: String,
    pub literal: bool,
}
impl DialogState {
    pub fn running(&self) -> bool {
        self.list_task.is_some() || self.task.is_some() || self.locks.running()
    }
    pub fn selected_entry(&self) -> Option<&LfsEntry> {
        self.status
            .as_ref()?
            .entries
            .iter()
            .find(|e| Some(&e.path) == self.selected.as_ref())
    }
}
impl Rocket {
    pub(crate) fn start_lfs_read(&mut self) {
        if self
            .dialog
            .as_ref()
            .and_then(|d| d.lfs.as_ref())
            .is_some_and(|s| s.locks_tab)
        {
            self.start_lfs_locks();
            return;
        }
        let Some(d) = &mut self.dialog else { return };
        if !matches!(d.kind, DialogKind::Lfs) {
            return;
        }
        let s = d.lfs.get_or_insert_with(Default::default);
        if s.applying {
            return;
        }
        s.list_task = None;
        s.task = None;
        s.preview = None;
        d.error = None;
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
        if let Some(action) = s.action.clone() {
            let source_scope = self.lfs_source_scope.clone();
            match ReadTask::spawn(move |cancel| {
                backend.lfs_preview(&path, action, source_scope.as_ref(), &cancel)
            }) {
                Ok(t) => s.task = Some(t),
                Err(e) => d.error = Some(user_error(&e)),
            }
        } else {
            match ReadTask::spawn(move |cancel| backend.lfs_status(&path, &cancel)) {
                Ok(t) => s.list_task = Some(t),
                Err(e) => d.error = Some(user_error(&e)),
            }
        }
    }
    pub(crate) fn poll_lfs_read(&mut self) {
        self.poll_lfs_locks();
        let Some(d) = &mut self.dialog else { return };
        let Some(s) = &mut d.lfs else { return };
        if let Some(result) = s.list_task.as_mut().and_then(ReadTask::try_finish) {
            s.list_task = None;
            match result {
                Ok(status) => {
                    if !status
                        .entries
                        .iter()
                        .any(|e| Some(&e.path) == s.selected.as_ref())
                    {
                        s.selected = None
                    }
                    if !status.remotes.contains(&s.remote) {
                        s.remote = status
                            .remotes
                            .iter()
                            .find(|r| r.as_str() == "origin")
                            .or(status.remotes.first())
                            .cloned()
                            .unwrap_or_default()
                    }
                    s.status = Some(status)
                }
                Err(e) => {
                    s.status = None;
                    s.selected = None;
                    d.error = Some(user_error(&e))
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
    }
    pub(crate) fn finish_lfs_dialog(&mut self, owner: Option<u64>, outcome: &Outcome) {
        if self
            .dialog
            .as_ref()
            .and_then(|d| d.lfs.as_ref())
            .is_some_and(|s| s.locks_tab)
        {
            self.finish_lfs_locks(owner, outcome);
            return;
        }
        let Some(d) = &mut self.dialog else { return };
        let Some(s) = &mut d.lfs else { return };
        if !s.applying || d.owner != owner {
            return;
        }
        s.applying = false;
        s.preview = None;
        s.action = None;
        s.notice = Some(format!(
            "{}\n{}",
            outcome.summary,
            outcome.details.join("\n")
        ));
        self.start_lfs_read();
    }
    /// Git LFS page: pointer files, tracking rules and locks. Every transfer,
    /// checkout, rule edit or lock change opens a review dialog.
    pub(crate) fn lfs_page(&mut self, ui: &mut Ui) {
        let writable = self.can_write();
        let Some(mut d) = self.dialog.take() else {
            return;
        };
        let s = d.lfs.get_or_insert_with(Default::default);
        let reviewing = s.applying
            || if s.locks_tab {
                s.locks.action.is_some()
            } else {
                s.action.is_some()
            };
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
            let description = s.status.as_ref().map_or_else(
                || "Large files stored outside Git history".to_owned(),
                |status| {
                    format!(
                        "{} · {} tracked files in the current index",
                        status.version,
                        status.entries.len()
                    )
                },
            );
            pages::header(ui, Icon::Database, "Git LFS", &description, |ui| {
                let r = pages::ghost(
                    ui,
                    !s.applying,
                    if waiting { "Cancel read" } else { "Close" },
                );
                control(widgets, "dialog-cancel", &r, ui.clip_rect());
                f.dismiss = r.clicked();
                let r = ui.add_enabled(
                    !waiting,
                    Button::new(if s.locks_tab {
                        "Refresh locks"
                    } else {
                        "Refresh"
                    }),
                );
                control(widgets, "dialog-preview-refresh", &r, ui.clip_rect());
                f.refresh |= r.clicked();
            });
            let count = s.status.as_ref().map(|status| status.entries.len());
            let rules = s
                .status
                .as_ref()
                .and_then(|status| status.tracking.as_ref().ok())
                .map(Vec::len);
            let locks = s.locks.list.as_ref().map(|list| list.entries.len());
            let tab = pages::tabs(
                ui,
                &[
                    pages::TabItem {
                        label: "Pointer files",
                        count,
                        selected: !s.locks_tab && !s.tracking,
                        enabled: !waiting,
                        key: "dialog-lfs-files",
                    },
                    pages::TabItem {
                        label: "Tracking rules",
                        count: rules,
                        selected: !s.locks_tab && s.tracking,
                        enabled: !waiting,
                        key: "dialog-lfs-tracking",
                    },
                    pages::TabItem {
                        label: "Locks",
                        count: locks,
                        selected: s.locks_tab,
                        enabled: !waiting,
                        key: "dialog-lfs-locks",
                    },
                ],
                widgets,
            );
            match tab {
                Some(0) => f.switch = Some(false),
                Some(1) => f.switch = Some(true),
                Some(2) if !s.locks_tab => f.locks = true,
                _ => {}
            }
            let body = ui.available_rect_before_wrap();
            if s.locks_tab {
                locks_tab(ui, s, &d.error, waiting, writable, widgets, &mut f);
            } else if s.tracking {
                tracking_tab(ui, s, &d.error, waiting, writable, widgets, &mut f);
            } else {
                files_tab(ui, s, &mut d.error, waiting, writable, widgets, &mut f);
            }
            pages::details_scroll(ui, widgets, !s.applying);
            pages::region(
                widgets,
                "dialog-lfs-scroll",
                body.intersect(ui.clip_rect()),
                !s.applying,
            );
        });
        self.finish_lfs_frame(d, f);
    }

    /// Review dialog over the Git LFS page. Also drops a page whose repository is
    /// no longer active.
    pub(crate) fn lfs_dialog(&mut self, ctx: &Context) {
        let writable = self.can_write();
        let Some(mut d) = self.dialog.take() else {
            return;
        };
        if d.owner != self.tab().map(|t| t.id) {
            return;
        }
        let s = d.lfs.get_or_insert_with(Default::default);
        if s.locks_tab {
            self.dialog = Some(d);
            self.lfs_lock_review(ctx);
            return;
        }
        let Some(action) = s.action.clone() else {
            self.dialog = Some(d);
            return;
        };
        let waiting = s.running() || s.applying;
        let mut f = Flags::default();
        let (image, title, subtitle) = match &action {
            LfsAction::Fetch(_) => (
                Icon::Fetch,
                "Download LFS objects",
                "Downloads the reviewed commit's objects into the cache. Files and staging are preserved.",
            ),
            LfsAction::Push(_) => (
                Icon::Push,
                "Upload LFS objects",
                "Uploads the objects in this commit's file tree. Publish the branch separately.",
            ),
            LfsAction::Checkout(_) => (
                Icon::Database,
                "Check out this LFS file",
                "Materializes one unchanged pointer. Staging is preserved.",
            ),
            LfsAction::Track { .. } => (
                Icon::Database,
                "Track a path or pattern",
                "Adds a rule to the root .gitattributes file.",
            ),
            LfsAction::Untrack { .. } => (
                Icon::Database,
                "Untrack this rule",
                "Removes the LFS fields from the selected root .gitattributes line.",
            ),
        };
        let head = pages::ReviewHeader {
            image,
            title,
            subtitle,
            danger: false,
        };
        let fraction = self.write.as_ref().and_then(|w| w.fraction);
        let running = pages::Running::of(self.write.as_ref());
        let response = pages::review(
            ctx,
            "lfs-dialog",
            head,
            &mut self.frame_widgets,
            "dialog-lfs-scroll",
            !s.applying,
            |ui, part, widgets| match part {
                pages::Part::Body => {
                    if let Some(p) = &s.preview {
                        pages::caption(ui, "What will happen");
                        match p.head() {
                            Head::Branch { name, target } => {
                                pages::field(ui, "Branch", &String::from_utf8_lossy(name), false);
                                pages::field(ui, "Commit", &target.to_string(), true);
                            }
                            Head::Detached { target } => {
                                pages::field(ui, "Detached", &target.to_string(), true)
                            }
                            Head::Unborn { .. } => {
                                pages::field(ui, "Commit", "No commit yet", false)
                            }
                        }
                        if let Some(source) = p.source() {
                            let label = if matches!(p.action(), LfsAction::Push(_)) {
                                "Destination"
                            } else {
                                "Source"
                            };
                            pages::field(ui, label, source, true);
                        }
                        if let LfsAction::Push(remote) = p.action() {
                            pages::field(ui, "Remote", remote, false);
                            pages::field(
                                ui,
                                "Objects",
                                &format!(
                                    "{} unique objects · {} bytes maximum",
                                    p.object_count(),
                                    p.total_bytes()
                                ),
                                false,
                            );
                            pages::callout(ui, GREEN(), "Local SHA-256 and sizes verified.");
                        }
                        for e in p.entries() {
                            pages::field(
                                ui,
                                "File",
                                &format!("{} · {} bytes", e.path.display_lossy(), e.object.size),
                                true,
                            );
                        }
                        if let Some(edit) = p.attributes() {
                            pages::note(
                                ui,
                                if edit.before().is_some() {
                                    "Existing .gitattributes · local edits included in recovery"
                                } else {
                                    "Create root .gitattributes"
                                },
                            );
                            if !edit.removed().is_empty() {
                                ui.add(
                                    Label::new(
                                        RichText::new(format!("− {}", edit.removed()))
                                            .monospace()
                                            .color(RED()),
                                    )
                                    .wrap(),
                                );
                            }
                            if !edit.added().is_empty() {
                                ui.add(
                                    Label::new(
                                        RichText::new(format!("+ {}", edit.added()))
                                            .monospace()
                                            .color(GREEN()),
                                    )
                                    .wrap(),
                                );
                            }
                        }
                    }
                    if let Some(notice) = &s.notice {
                        pages::callout(ui, BLUE(), notice);
                    }
                    // A running operation has its status and Stop in the footer; only
                    // its measured progress shows here.
                    if waiting && !s.applying {
                        pages::waiting(ui, "Reading LFS state and validating the preview…");
                    }
                    if s.applying
                        && let Some(fraction) = fraction
                    {
                        ui.add(ProgressBar::new(fraction).show_percentage());
                    }
                    if let Some(error) = &d.error {
                        pages::callout(ui, RED(), error);
                    }
                }
                pages::Part::Footer if s.applying => {
                    f.cancel = pages::running_footer(ui, widgets, &running, &[]);
                }
                pages::Part::Footer => {
                    let label = match s.action {
                        Some(LfsAction::Fetch(_)) => "Fetch objects",
                        Some(LfsAction::Push(_)) => "Upload objects",
                        Some(LfsAction::Track { .. }) => "Track rule",
                        Some(LfsAction::Untrack { .. }) => "Untrack rule",
                        _ => "Check out file",
                    };
                    let r = pages::primary(
                        ui,
                        !waiting && writable && s.preview.is_some() && d.error.is_none(),
                        label,
                    );
                    control(widgets, "dialog-submit", &r, ui.clip_rect());
                    f.submit = r.clicked();
                    let r = ui.add_enabled(!waiting, Button::new("Review again"));
                    control(widgets, "dialog-preview-refresh", &r, ui.clip_rect());
                    f.refresh |= r.clicked();
                }
            },
        );
        // The × returns to the Git LFS page; automation's old Cancel id is that ×.
        if let Some(close) = self.frame_widgets.get("dialog-close").copied() {
            self.frame_widgets.insert("dialog-lfs-back".into(), close);
        }
        f.back |= response.should_close() && !s.applying;
        self.finish_lfs_frame(d, f);
    }

    /// Apply one frame's page or review choices for every LFS tab.
    pub(crate) fn finish_lfs_frame(&mut self, mut d: Dialog, mut f: Flags) {
        let s = d.lfs.get_or_insert_with(Default::default);
        if let Some(tracking) = f.switch {
            s.locks_tab = false;
            s.tracking = tracking;
            d.error = None;
        }
        if f.locks {
            s.locks_tab = true;
            s.locks.path = s
                .selected
                .as_ref()
                .map(|p| p.display_lossy().to_string())
                .unwrap_or_default();
            s.notice = None;
            f.refresh = true;
        }
        if f.back {
            if s.locks_tab {
                let l = &mut s.locks;
                l.action = None;
                l.preview = None;
                l.task = None;
                l.force_confirmed = false;
                d.error = None;
            } else {
                s.action = None;
                s.preview = None;
                s.task = None;
                f.refresh = true;
            }
        }
        if f.cancel {
            self.cancel_write();
        }
        if f.submit {
            let action = if s.locks_tab {
                s.locks.preview.clone().map(Action::LfsLock)
            } else {
                s.preview.clone().map(Action::Lfs)
            };
            if let (Some(owner), Some(action)) = (d.owner, action) {
                s.applying = true;
                self.execute(owner, action);
                if self.write.is_none() {
                    s.applying = false;
                    d.error = Some("Could not start. Refresh and review again.".into());
                }
            }
        }
        if !f.dismiss {
            self.dialog = Some(d);
        }
        if f.refresh {
            self.start_lfs_read();
        }
    }
}

#[derive(Default)]
pub(crate) struct Flags {
    pub dismiss: bool,
    pub refresh: bool,
    pub submit: bool,
    pub back: bool,
    pub cancel: bool,
    /// Leave the Locks tab for Pointer files (false) or Tracking rules (true).
    pub switch: Option<bool>,
    pub locks: bool,
}

fn notices(ui: &mut Ui, s: &DialogState, error: &Option<String>) {
    if s.notice.is_none() && error.is_none() {
        return;
    }
    pages::toolbar(ui, |ui| {
        ui.vertical(|ui| {
            if let Some(notice) = &s.notice {
                pages::callout(ui, BLUE(), notice);
            }
            if let Some(error) = error {
                pages::callout(ui, RED(), error);
            }
        });
    });
}

fn files_tab(
    ui: &mut Ui,
    s: &mut DialogState,
    error: &mut Option<String>,
    waiting: bool,
    writable: bool,
    widgets: &mut HashMap<String, UiControl>,
    f: &mut Flags,
) {
    pages::toolbar(ui, |ui| {
        if let Some(status) = &s.status {
            ui.label(RichText::new("Remote").color(FAINT()));
            for (i, name) in status.remotes.iter().enumerate() {
                let r = pages::choice(ui, !waiting, &s.remote == name, name);
                control(
                    widgets,
                    format!("dialog-lfs-remote-{i}"),
                    &r,
                    ui.clip_rect(),
                );
                if r.clicked() {
                    s.remote = name.clone();
                }
            }
            if status.remotes.is_empty() {
                ui.label(RichText::new("No remotes").color(FAINT()));
            }
        }
        ui.add_space(8.0);
        let enabled = !waiting && writable && s.status.is_some() && !s.remote.is_empty();
        let r = ui.add_enabled(enabled, Button::new("Review fetch"));
        control(widgets, "dialog-lfs-fetch", &r, ui.clip_rect());
        if r.clicked() {
            s.action = Some(LfsAction::Fetch(s.remote.clone()));
            s.notice = None;
            f.refresh = true;
        }
        let r = ui.add_enabled(enabled, Button::new("Review upload"));
        control(widgets, "dialog-lfs-push", &r, ui.clip_rect());
        if r.clicked() {
            s.action = Some(LfsAction::Push(s.remote.clone()));
            s.notice = None;
            f.refresh = true;
        }
    });
    notices(ui, s, error);
    pages::split(ui, |ui, side| match side {
        pages::Side::List => {
            let height = (ui.available_height() - 52.0).max(80.0);
            ScrollArea::vertical()
                .id_salt("lfs-files")
                .max_height(height)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    let Some(status) = &s.status else {
                        if waiting {
                            pages::loading(ui, "Reading LFS pointers…");
                        }
                        return;
                    };
                    if status.entries.is_empty() {
                        pages::empty(ui, "No LFS pointer files are tracked in this index.");
                    }
                    for (i, e) in status.entries.iter().enumerate() {
                        let path = e.path.display_lossy().to_string();
                        let detail = format!("{} bytes", e.object.size);
                        let r = pages::row(
                            ui,
                            pages::Row {
                                image: Icon::Database,
                                title: &path,
                                detail: &detail,
                                pills: vec![
                                    (
                                        e.state.label().to_string(),
                                        if e.state == LfsFileState::Pointer {
                                            pages::WARN()
                                        } else {
                                            pages::OK()
                                        },
                                    ),
                                    (
                                        if e.cached {
                                            "Cached".into()
                                        } else {
                                            "Not cached".into()
                                        },
                                        if e.cached {
                                            pages::INFO()
                                        } else {
                                            pages::NEUTRAL()
                                        },
                                    ),
                                ],
                                indent: 0.0,
                            },
                            Some(&e.path) == s.selected.as_ref(),
                            !waiting,
                        );
                        control(widgets, format!("dialog-lfs-row-{i}"), &r, ui.clip_rect());
                        if r.clicked() {
                            s.selected = Some(e.path.clone());
                            *error = None;
                        }
                    }
                });
        }
        pages::Side::Details => {
            let Some(e) = s.selected_entry().cloned() else {
                pages::empty(ui, "Select a file to see its object and check it out.");
                return;
            };
            ui.add(Label::new(RichText::new(e.path.display_lossy()).font(semibold(15.0))).wrap());
            pages::field(ui, "Object", &e.object.oid.to_string(), true);
            pages::field(ui, "Size", &format!("{} bytes", e.object.size), false);
            pages::field(ui, "State", e.state.label(), false);
            pages::field(
                ui,
                "Cache",
                if e.cached {
                    "Cached locally"
                } else {
                    "Not cached"
                },
                false,
            );
            ui.add_space(8.0);
            let enabled = !waiting && writable && e.state == LfsFileState::Pointer && e.cached;
            let r = pages::primary(ui, enabled, "Review checkout…");
            control(widgets, "dialog-lfs-checkout", &r, ui.clip_rect());
            if r.clicked() {
                s.action = Some(LfsAction::Checkout(e.path.clone()));
                s.notice = None;
                f.refresh = true;
            }
        }
    });
}

fn tracking_tab(
    ui: &mut Ui,
    s: &mut DialogState,
    error: &Option<String>,
    waiting: bool,
    writable: bool,
    widgets: &mut HashMap<String, UiControl>,
    f: &mut Flags,
) {
    let rules_ok = s
        .status
        .as_ref()
        .is_some_and(|status| status.tracking.is_ok());
    pages::toolbar(ui, |ui| {
        if rules_ok {
            let r = ui.add_enabled(
                !waiting,
                TextEdit::singleline(&mut s.pattern)
                    .desired_width((ui.available_width() - 330.0).clamp(160.0, 420.0))
                    .hint_text("assets/*.png"),
            );
            control(widgets, "dialog-lfs-pattern", &r, ui.clip_rect());
            let r = ui.add_enabled(
                !waiting,
                checkbox(&mut s.literal, "Treat as a literal file path"),
            );
            control(widgets, "dialog-lfs-literal", &r, ui.clip_rect());
        }
        let enabled = !waiting && writable && !s.pattern.is_empty() && rules_ok;
        let r = pages::primary(ui, enabled, "Review track…");
        control(widgets, "dialog-lfs-track", &r, ui.clip_rect());
        if r.clicked() {
            s.action = Some(LfsAction::Track {
                pattern: s.pattern.clone(),
                literal: s.literal,
            });
            s.notice = None;
            f.refresh = true;
        }
    });
    notices(ui, s, error);
    Frame::new()
        .inner_margin(Margin { left: 24, right: 24, top: 4, bottom: 12 })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            let Some(status) = &s.status else {
                if waiting {
                    pages::loading(ui, "Reading root .gitattributes…");
                }
                return;
            };
            match &status.tracking {
                Ok(rules) => {
                    pages::caption(ui, &format!("Root .gitattributes · {} explicit LFS rules", rules.len()));
                    ui.add_space(4.0);
                    let height = (ui.available_height() - 90.0).max(80.0);
                    ScrollArea::vertical().id_salt("lfs-rules").max_height(height).auto_shrink([false, true]).show(ui, |ui| {
                        if rules.is_empty() {
                            pages::empty(ui, "No explicit LFS rules yet. Track a path or pattern above.");
                        }
                        for (i, rule) in rules.iter().enumerate() {
                            Frame::new()
                                .fill(PANEL())
                                .stroke(Stroke::new(1.0, HAIRLINE()))
                                .corner_radius(8)
                                .inner_margin(Margin::symmetric(12, 8))
                                .show(ui, |ui| {
                                    ui.set_width(ui.available_width());
                                    ui.horizontal(|ui| {
                                        ui.label(RichText::new(format!("Line {}", rule.line)).size(12.0).color(FAINT()));
                                        ui.add(Label::new(RichText::new(&rule.text).monospace()).truncate());
                                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                            let r = ui.add_enabled(!waiting && writable, Button::new("Review untrack"));
                                            control(widgets, format!("dialog-lfs-untrack-{i}"), &r, ui.clip_rect());
                                            if r.clicked() {
                                                s.action = Some(LfsAction::Untrack { line: rule.line, expected: rule.text.clone() });
                                                s.notice = None;
                                                f.refresh = true;
                                            }
                                        });
                                    });
                                });
                            ui.add_space(4.0);
                        }
                    });
                }
                Err(e) => pages::callout(ui, RED(), e),
            }
        });
}

fn locks_tab(
    ui: &mut Ui,
    s: &mut DialogState,
    error: &Option<String>,
    waiting: bool,
    writable: bool,
    widgets: &mut HashMap<String, UiControl>,
    f: &mut Flags,
) {
    pages::toolbar(ui, |ui| {
        ui.label(RichText::new("Remote").color(FAINT()));
        ComboBox::from_id_salt("lfs-lock-remote")
            .selected_text(&s.remote)
            .show_ui(ui, |ui| {
                if let Some(status) = &s.status {
                    for (i, name) in status.remotes.iter().enumerate() {
                        let r =
                            ui.add_enabled(!waiting, Button::new(name).selected(s.remote == *name));
                        control(
                            widgets,
                            format!("dialog-lfs-lock-remote-{i}"),
                            &r,
                            ui.clip_rect(),
                        );
                        if r.clicked() {
                            s.remote = name.clone();
                            s.locks.selected = None;
                            s.notice = None;
                            f.refresh = true;
                        }
                    }
                }
            });
        ui.add_space(8.0);
        let r = ui.add_enabled(
            !waiting,
            TextEdit::singleline(&mut s.locks.path)
                .desired_width((ui.available_width() - 140.0).clamp(160.0, 380.0))
                .hint_text("assets/background.dat"),
        );
        control(widgets, "dialog-lfs-lock-path", &r, ui.clip_rect());
        let enabled = !waiting && writable && !s.remote.is_empty() && s.locks.list.is_some();
        let r = pages::primary(ui, enabled && !s.locks.path.is_empty(), "Review lock…");
        control(widgets, "dialog-lfs-lock-create", &r, ui.clip_rect());
        if r.clicked() {
            s.locks.action = Some(LfsLockAction::Lock(s.locks.path.clone()));
            s.notice = None;
            f.refresh = true;
        }
    });
    notices(ui, s, error);
    pages::split(ui, |ui, side| match side {
        pages::Side::List => {
            let Some(list) = &s.locks.list else {
                if waiting {
                    pages::loading(ui, "Reading server locks and verifying ownership…");
                } else {
                    pages::empty(
                        ui,
                        "Locks have not been verified. Refresh to read the server.",
                    );
                }
                return;
            };
            pages::caption(
                ui,
                &format!(
                    "{} server-verified locks · refresh before acting on older results",
                    list.entries.len()
                ),
            );
            ui.add_space(4.0);
            if list.entries.is_empty() {
                pages::empty(ui, "No locks on this remote.");
            }
            let height = (ui.available_height() - 40.0).max(80.0);
            ScrollArea::vertical()
                .id_salt("lfs-lock-rows")
                .max_height(height)
                .auto_shrink([false, true])
                .show_rows(ui, 48.0, list.entries.len(), |ui, range| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    for i in range {
                        let entry = &list.entries[i];
                        let owner = if entry.owner.is_empty() {
                            "Owner unavailable"
                        } else {
                            &entry.owner
                        };
                        let detail = format!("{owner} · {}", entry.locked_at);
                        let r = pages::row(
                            ui,
                            pages::Row {
                                image: Icon::Lock,
                                title: &entry.path,
                                detail: &detail,
                                pills: vec![if entry.ours {
                                    ("You".into(), pages::OK())
                                } else {
                                    (owner.to_string(), pages::WARN())
                                }],
                                indent: 0.0,
                            },
                            s.locks.selected.as_ref() == Some(&entry.id),
                            true,
                        );
                        control(
                            widgets,
                            format!("dialog-lfs-lock-row-{i}"),
                            &r,
                            ui.clip_rect(),
                        );
                        if r.clicked() {
                            s.locks.selected = Some(entry.id.clone());
                        }
                    }
                });
        }
        pages::Side::Details => {
            let Some(entry) = s.locks.selected_entry().cloned() else {
                pages::empty(ui, "Select a lock to unlock it or review a force unlock.");
                return;
            };
            ui.add(Label::new(RichText::new(&entry.path).font(semibold(15.0))).wrap());
            pages::field(
                ui,
                "Owner",
                if entry.ours {
                    "You"
                } else if entry.owner.is_empty() {
                    "Unavailable"
                } else {
                    &entry.owner
                },
                false,
            );
            pages::field(ui, "Lock ID", &entry.id, true);
            pages::field(ui, "Locked", &entry.locked_at.to_string(), false);
            ui.add_space(8.0);
            let enabled = !waiting && writable && !s.remote.is_empty() && s.locks.list.is_some();
            ui.horizontal_wrapped(|ui| {
                let r = pages::primary(ui, enabled && entry.ours, "Review unlock…");
                control(widgets, "dialog-lfs-lock-unlock", &r, ui.clip_rect());
                if r.clicked() {
                    s.locks.action = Some(LfsLockAction::Unlock {
                        lock: entry.clone(),
                        force: false,
                    });
                    s.notice = None;
                    f.refresh = true;
                }
                let r = pages::danger_ghost(ui, enabled && !entry.ours, "Review force unlock…");
                control(widgets, "dialog-lfs-lock-force", &r, ui.clip_rect());
                if r.clicked() {
                    s.locks.action = Some(LfsLockAction::Unlock {
                        lock: entry.clone(),
                        force: true,
                    });
                    s.notice = None;
                    f.refresh = true;
                }
            });
            pages::note(
                ui,
                if entry.ours {
                    "Unlocking does not change your local files."
                } else {
                    "Force unlock removes another account's lock and needs a separate confirmation."
                },
            );
        }
    });
}
