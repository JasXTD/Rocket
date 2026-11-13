//! Submodule list and owner-bound, cancellable checkout previews.
use crate::{
    app::*,
    backend::{Action, Outcome, user_error},
    pages::{self, control},
    style::*,
};
use eframe::egui::*;
use rocket_git::*;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum ManagementMode {
    #[default]
    Browse,
    Add,
    Remove,
    Deinitialize,
    Sync,
    SetUrl,
    Move,
}
impl ManagementMode {
    pub fn name(self) -> &'static str {
        match self {
            Self::Browse => "browse",
            Self::Add => "add",
            Self::Remove => "remove",
            Self::Deinitialize => "deinitialize",
            Self::Sync => "sync",
            Self::SetUrl => "set-url",
            Self::Move => "move",
        }
    }
}

#[derive(Default)]
pub struct DialogState {
    pub mode: ManagementMode,
    pub add_parent: Option<PathBuf>,
    pub source: String,
    pub new_path: String,
    pub action_preview: Option<Arc<SubmoduleActionPreview>>,
    pub action_task: Option<ReadTask<SubmoduleActionPreview>>,
    pub entries: Vec<SubmoduleEntry>,
    pub selected: Option<RepoPath>,
    pub preview: Option<Arc<SubmodulePreview>>,
    pub list_task: Option<ReadTask<Vec<SubmoduleEntry>>>,
    pub task: Option<ReadTask<SubmodulePreview>>,
    pub open_task: Option<ReadTask<PathBuf>>,
    pub reviewing: bool,
    pub applying: bool,
    pub notice: Option<String>,
}
impl DialogState {
    pub fn running(&self) -> bool {
        self.list_task.is_some()
            || self.task.is_some()
            || self.open_task.is_some()
            || self.action_task.is_some()
    }
    pub fn selected_entry(&self) -> Option<&SubmoduleEntry> {
        self.entries
            .iter()
            .find(|e| Some(&e.path) == self.selected.as_ref())
    }
}
impl Rocket {
    pub(crate) fn start_submodule_read(&mut self) {
        let Some(d) = &mut self.dialog else { return };
        if !matches!(d.kind, DialogKind::Submodules) {
            return;
        }
        let s = d.submodules.get_or_insert_with(Default::default);
        if s.applying {
            return;
        }
        s.list_task = None;
        s.task = None;
        s.preview = None;
        s.action_task = None;
        s.action_preview = None;
        s.open_task = None;
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
        if s.mode != ManagementMode::Browse {
            let fixture_root = self.submodule_fixture_root.clone();
            let action = match s.mode {
                ManagementMode::Add => {
                    RepoPath::new(s.new_path.as_bytes().to_vec()).map(|relative| {
                        SubmoduleAction::Add {
                            parent: s.add_parent.clone().unwrap_or_else(|| path.clone()),
                            path: relative,
                            source: s.source.clone(),
                        }
                    })
                }
                ManagementMode::Remove => s
                    .selected
                    .clone()
                    .map(SubmoduleAction::Remove)
                    .ok_or_else(|| Error::InvalidInput(crate::i18n::tr("Select a submodule to remove").into())),
                ManagementMode::Deinitialize=>s.selected.clone().map(SubmoduleAction::Deinitialize).ok_or_else(||Error::InvalidInput(crate::i18n::tr("Select a submodule to deinitialize").into())),
                ManagementMode::Sync => s
                    .selected
                    .clone()
                    .map(SubmoduleAction::Sync)
                    .ok_or_else(|| Error::InvalidInput(crate::i18n::tr("Select a submodule to sync").into())),
                ManagementMode::SetUrl => s.selected.clone()
                    .map(|path| SubmoduleAction::SetUrl { path, source: s.source.clone() })
                    .ok_or_else(|| Error::InvalidInput(crate::i18n::tr("Select a submodule to edit").into())),
                ManagementMode::Move => s.selected.clone()
                    .ok_or_else(|| Error::InvalidInput(crate::i18n::tr("Select a submodule to move").into()))
                    .and_then(|path| Ok(SubmoduleAction::Move { path, destination: RepoPath::new(s.new_path.as_bytes().to_vec())? })),
                _ => unreachable!(),
            };
            match action.and_then(|action| {
                ReadTask::spawn(move |cancel| {
                    backend.submodule_action_preview(
                        &path,
                        &action,
                        fixture_root.as_deref(),
                        &cancel,
                    )
                })
            }) {
                Ok(t) => s.action_task = Some(t),
                Err(e) => d.error = Some(user_error(&e)),
            }
        } else if s.reviewing {
            let Some(e) = s.selected_entry().cloned() else {
                return;
            };
            match ReadTask::spawn(move |cancel| backend.submodule_preview(&path, &e, &cancel)) {
                Ok(t) => s.task = Some(t),
                Err(e) => d.error = Some(user_error(&e)),
            }
        } else {
            match ReadTask::spawn(move |cancel| backend.submodules(&path, &cancel)) {
                Ok(t) => s.list_task = Some(t),
                Err(e) => d.error = Some(user_error(&e)),
            }
        }
    }
    pub(crate) fn poll_submodule_read(&mut self) {
        let active = self.tab().map(|t| t.id);
        let Some(d) = &mut self.dialog else { return };
        let Some(s) = &mut d.submodules else { return };
        if let Some(r) = s.list_task.as_mut().and_then(ReadTask::try_finish) {
            s.list_task = None;
            match r {
                Ok(entries) => {
                    if !entries.iter().any(|e| Some(&e.path) == s.selected.as_ref()) {
                        s.selected = None
                    }
                    s.entries = entries;
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
        if let Some(r) = s.action_task.as_mut().and_then(ReadTask::try_finish) {
            s.action_task = None;
            match r {
                Ok(p) => s.action_preview = Some(Arc::new(p)),
                Err(e) => d.error = Some(user_error(&e)),
            }
        }
        let opened = s.open_task.as_mut().and_then(ReadTask::try_finish);
        if let Some(r) = opened {
            s.open_task = None;
            match r {
                Err(e) => d.error = Some(user_error(&e)),
                Ok(path) if d.owner == active => {
                    self.dialog = None;
                    if let Some(i) = self.tabs.iter().position(|t| {
                        t.snapshot
                            .as_ref()
                            .and_then(|s| s.info.worktree.as_ref())
                            .unwrap_or(&t.path)
                            == &path
                    }) {
                        self.active = i
                    } else {
                        self.open(path)
                    }
                }
                _ => {}
            }
        }
    }
    pub(crate) fn finish_submodule_dialog(&mut self, owner: Option<u64>, outcome: &Outcome) {
        let Some(d) = &mut self.dialog else { return };
        let Some(s) = &mut d.submodules else { return };
        if !s.applying || owner != d.owner {
            return;
        }
        s.applying = false;
        s.reviewing = false;
        s.preview = None;
        s.action_preview = None;
        s.mode = ManagementMode::Browse;
        s.notice = Some(format!(
            "{}\n{}",
            outcome.summary,
            outcome.details.join("\n")
        ));
        self.start_submodule_read();
    }
    /// Submodules page: list and details. Every change opens the review dialog.
    pub(crate) fn submodule_page(&mut self, ui: &mut Ui) {
        let writable = self.can_write();
        let Some(mut d) = self.dialog.take() else {
            return;
        };
        let s = d.submodules.get_or_insert_with(Default::default);
        let reviewing = s.reviewing || s.mode != ManagementMode::Browse || s.applying;
        let waiting = s.running() || s.applying;
        let mut f = Flags::default();
        let command_context = self.tabs.iter().find(|t|Some(t.id)==d.owner).map(crate::custom_commands::context);
        let mut command_intents=Vec::new();
        let post_switch = &mut self.review_submodules_after_switch;
        // Under a review dialog the page is inert and publishes no controls.
        let mut inert = HashMap::new();
        let widgets = if reviewing {
            &mut inert
        } else {
            &mut self.frame_widgets
        };
        ui.add_enabled_ui(!reviewing, |ui| {
            pages::header(
                ui,
                Icon::Modules,
                crate::i18n::tr("Submodules"),
                &crate::i18n::trf!(
                    "Recorded checkouts: {count} · parent staging is never changed by browsing",
                    count = s.entries.len()
                ),
                |ui| {
                    let r = pages::ghost(
                        ui,
                        !s.applying,
                        if s.running() { crate::i18n::tr("Cancel read") } else { crate::i18n::tr("Close") },
                    );
                    control(widgets, "dialog-cancel", &r, ui.clip_rect());
                    f.dismiss = r.clicked();
                    let r = pages::primary(ui, !waiting && writable, crate::i18n::tr("Add submodule"));
                    control(widgets, "dialog-submodule-add", &r, ui.clip_rect());
                    if r.clicked() {
                        s.mode = ManagementMode::Add;
                        s.add_parent = None;
                        s.preview = None;
                        s.action_preview = None;
                        s.notice = None;
                        d.error = None;
                    }
                    let r = ui.add_enabled(!waiting, Button::new(crate::i18n::tr("Refresh")));
                    control(widgets, "dialog-preview-refresh", &r, ui.clip_rect());
                    f.refresh |= r.clicked();
                },
            );
            pages::toolbar(ui,|ui|{let r=ui.checkbox(post_switch,crate::i18n::tr("Review submodules after branch switching (this session)"));control(widgets,"dialog-submodule-after-switch",&r,ui.clip_rect());});
            let line = ui
                .allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover())
                .0;
            ui.painter()
                .hline(line.x_range(), line.center().y, Stroke::new(1.0, HAIRLINE()));
            if !reviewing && (d.error.is_some() || s.notice.is_some()) {
                pages::toolbar(ui, |ui| {
                    ui.vertical(|ui| {
                        if let Some(notice) = &s.notice {
                            pages::callout(ui, BLUE(), notice);
                        }
                        if let Some(error) = &d.error {
                            pages::callout(ui, RED(), error);
                        }
                    });
                });
            }
            let body = ui.available_rect_before_wrap();
            pages::split(ui, |ui, side| match side {
                pages::Side::List => {
                    let height = (ui.available_height() - 8.0).max(80.0);
                    let list = ScrollArea::vertical()
                        .id_salt("submodules-list")
                        .max_height(height)
                        .auto_shrink([false, true])
                        .show(ui, |ui| {
                            ui.spacing_mut().item_spacing.y = 2.0;
                            if s.entries.is_empty() && !waiting && d.error.is_none() {
                                pages::empty(
                                    ui,
                                    crate::i18n::tr("No submodules recorded in this repository index."),
                                );
                            }
                            if s.entries.is_empty() && waiting {
                                pages::loading(ui, crate::i18n::tr("Reading submodules…"));
                            }
                            for (i, e) in s.entries.iter().enumerate() {
                                let title = e.path.display_lossy().to_string();
                                let detail = crate::i18n::trf!("recorded {commit}", commit = &e.target.as_str()[..8]);
                                let state = state_label(e).to_string();
                                let color = if e.blocked.is_some() || e.dirty {
                                    pages::WARN()
                                } else if e.initialized {
                                    pages::OK()
                                } else {
                                    pages::NEUTRAL()
                                };
                                let r = pages::row(
                                    ui,
                                    pages::Row {
                                        image: Icon::Modules,
                                        title: &title,
                                        detail: &detail,
                                        pills: vec![(state, color)],
                                        indent: e.depth as f32 * 18.0,
                                    },
                                    Some(&e.path) == s.selected.as_ref(),
                                    !waiting,
                                );
                                control(
                                    widgets,
                                    format!("dialog-submodule-row-{i}"),
                                    &r,
                                    ui.clip_rect(),
                                );
                                r.context_menu(|ui| {
                                    if let Some(mut context)=command_context.clone() {
                                        context.submodule=if e.initialized && e.blocked.is_none() { std::str::from_utf8(e.path.as_bytes()).ok().map(|p|context.repository.join(p)) }else{None};
                                        crate::custom_commands::context_menu(ui,&self.commands,rocket_git::CommandScope::Submodule,&context,&mut command_intents,widgets);
                                    }
                                });
                                if r.clicked() {
                                    s.selected = Some(e.path.clone());
                                    s.preview = None;
                                    d.error = None;
                                    s.notice = None;
                                }
                            }
                        });
                    pages::region(
                        widgets,
                        "dialog-submodule-list-scroll",
                        list.inner_rect.intersect(ui.clip_rect()),
                        !waiting,
                    );
                }
                pages::Side::Details => {
                    let Some(e) = s.selected_entry().cloned() else {
                        pages::empty(
                            ui,
                            crate::i18n::tr("Select a submodule to see its source, recorded commit and checkout."),
                        );
                        return;
                    };
                    entry_details(ui, &e);
                    ui.add_space(10.0);
                    ui.horizontal_wrapped(|ui| {
                        let r = pages::primary(
                            ui,
                            !waiting && writable,
                            if e.initialized {
                                crate::i18n::tr("Review update")
                            } else {
                                crate::i18n::tr("Review initialization")
                            },
                        );
                        control(widgets, "dialog-submodule-review", &r, ui.clip_rect());
                        if r.clicked() {
                            s.reviewing = true;
                            s.notice = None;
                            f.refresh = true;
                        }
                        let r = ui
                            .add_enabled(!waiting && e.initialized, Button::new(crate::i18n::tr("Open repository")));
                        control(widgets, "dialog-submodule-open", &r, ui.clip_rect());
                        f.opening = r.clicked();
                    });
                    ui.horizontal_wrapped(|ui| {
                        let r = ui.add_enabled(!waiting && writable, Button::new(crate::i18n::tr("Edit URL")));
                        control(widgets, "dialog-submodule-edit-url", &r, ui.clip_rect());
                        if r.clicked() { f.mode = Some(ManagementMode::SetUrl); }
                        let r = ui.add_enabled(!waiting && writable, Button::new(crate::i18n::tr("Sync URL")));
                        control(widgets, "dialog-submodule-sync", &r, ui.clip_rect());
                        if r.clicked() {
                            f.mode = Some(ManagementMode::Sync);
                        }
                        let r = pages::danger_ghost(ui, !waiting && writable, crate::i18n::tr("Remove"));
                        // Deinitialization is a separate reviewed operation; it retains the gitlink.
                        control(widgets, "dialog-submodule-remove", &r, ui.clip_rect());
                        if r.clicked() {
                            f.mode = Some(ManagementMode::Remove);
                        }
                        let r=ui.add_enabled(!waiting&&writable&&e.initialized,Button::new(crate::i18n::tr("Deinitialize")));
                        control(widgets,"dialog-submodule-deinitialize",&r,ui.clip_rect());
                        if r.clicked(){f.mode=Some(ManagementMode::Deinitialize);}
                        let r = ui.add_enabled(!waiting && writable, Button::new(crate::i18n::tr("Move / rename…")));
                        control(widgets, "dialog-submodule-move", &r, ui.clip_rect());
                        if r.clicked() { f.mode = Some(ManagementMode::Move); }
                    });
                }
            });
            pages::details_scroll(ui, widgets, true);
            pages::region(
                widgets,
                "dialog-submodules-scroll",
                body.intersect(ui.clip_rect()),
                true,
            );
        });
        self.finish_submodule_frame(d, f);
        for intent in command_intents {self.dispatch(intent);}
    }

    /// Review dialog over the Submodules page, one layout for every operation: the
    /// submodule it acts on, its inputs, then what the review found, each change as the
    /// old value struck through and the new one. Also drops a page whose repository is
    /// no longer active.
    pub(crate) fn submodule_dialog(&mut self, ctx: &Context) {
        let writable = self.can_write();
        let open_paths = self
            .tabs
            .iter()
            .map(|t| {
                t.snapshot
                    .as_ref()
                    .and_then(|s| s.info.worktree.clone())
                    .unwrap_or_else(|| t.path.clone())
            })
            .collect::<Vec<_>>();
        let Some(mut d) = self.dialog.take() else {
            return;
        };
        let repository = self
            .tabs
            .iter()
            .find(|t| Some(t.id) == d.owner)
            .map(|t| t.title.clone())
            .unwrap_or_default();
        let s = d.submodules.get_or_insert_with(Default::default);
        if d.owner != self.tab().map(|t| t.id) && !s.applying {
            return;
        }
        if !s.reviewing && s.mode == ManagementMode::Browse && !s.applying {
            self.dialog = Some(d);
            return;
        }
        let waiting = s.running() || s.applying;
        let mut f = Flags::default();
        let mut browse = false;
        let mode = s.mode;
        let entry = s.selected_entry().cloned();
        let (title, subtitle, danger) = review_heading(mode, entry.as_ref(), &repository);
        let head = pages::ReviewHeader {
            image: Icon::Modules,
            title: &title,
            subtitle: &subtitle,
            danger,
        };
        let running = pages::Running::of(self.write.as_ref());
        let remove_open = matches!(mode, ManagementMode::Remove | ManagementMode::Deinitialize | ManagementMode::Move)
            && entry
                .as_ref()
                .is_some_and(|e| open_paths.iter().any(|p| p.starts_with(&e.directory)));
        let response = pages::review(
            ctx,
            "rocket-submodules",
            head,
            &mut self.frame_widgets,
            "dialog-submodules-scroll",
            true,
            |ui, part, widgets| match part {
                pages::Part::Body => {
                    ui.spacing_mut().item_spacing.y = 12.0;
                    if mode == ManagementMode::Add {
                        add_inputs(ui, s, &mut d.error, widgets, waiting, &mut browse);
                    } else if let Some(e) = &entry {
                        module_card(ui, e);
                    }
                    if mode == ManagementMode::SetUrl
                        && source_input(ui, s, widgets, crate::i18n::tr("New URL or local repository"), &mut browse)
                    {
                        s.action_task = None;
                        s.action_preview = None;
                        d.error = None;
                    }
                    if let Some(p) = &s.preview {
                        checkout_review(ui, p);
                    }
                    if mode == ManagementMode::Move {
                        field_name(ui, crate::i18n::tr("New path in the parent repository"));
                        let r = ui.add_enabled(!waiting, TextEdit::singleline(&mut s.new_path).desired_width(ui.available_width()).hint_text("components/library"));
                        control(widgets, "dialog-submodule-path", &r, ui.clip_rect());
                        if r.changed() { s.action_preview = None; d.error = None; }
                        pages::note(ui, crate::i18n::tr("Choose an unused path with an existing parent folder. The checkout and nested submodules must be clean. Module names, URLs and history are kept."));
                    }
                    if let Some(p) = &s.action_preview {
                        action_review(ui, p, entry.as_ref());
                    }
                    if remove_open {
                        banner(ui, AMBER(), crate::i18n::tr("Close this submodule's repository tabs before moving or removing it."), None);
                    }
                    if let Some(error) = &d.error {
                        let (code, message) = split_error(error);
                        banner(ui, RED(), message, code);
                    }
                }
                pages::Part::Footer if s.applying => {
                    f.cancel = pages::running_footer(ui, widgets, &running, &[]);
                }
                pages::Part::Footer => {
                    let reviewed = s.preview.is_some() || s.action_preview.is_some();
                    let label = match mode {
                        ManagementMode::Add => crate::i18n::tr("Add submodule"),
                        ManagementMode::Remove => crate::i18n::tr("Remove submodule"),
                        ManagementMode::Deinitialize => crate::i18n::tr("Deinitialize"),
                        ManagementMode::Sync => crate::i18n::tr("Sync URL"),
                        ManagementMode::SetUrl => crate::i18n::tr("Update URL and sync"),
                        ManagementMode::Move => crate::i18n::tr("Move submodule"),
                        _ => s.preview.as_ref().map_or(crate::i18n::tr("Apply checkout"), |p| {
                            if p.entry.initialized {
                                crate::i18n::tr("Update submodule")
                            } else {
                                crate::i18n::tr("Initialize submodule")
                            }
                        }),
                    };
                    let ready = !waiting && writable && !remove_open && reviewed && d.error.is_none();
                    // Added right to left: the reviewed operation by name, Review (again).
                    if reviewed || d.error.is_some() {
                        let r = if matches!(mode, ManagementMode::Remove | ManagementMode::Deinitialize) {
                            pages::danger(ui, ready, label)
                        } else {
                            pages::primary(ui, ready, label)
                        };
                        control(widgets, "dialog-submit", &r, ui.clip_rect());
                        f.submit = r.clicked();
                    }
                    let r = if reviewed || d.error.is_some() {
                        ui.add_enabled(!waiting, Button::new(crate::i18n::tr("Review again")))
                    } else {
                        pages::primary(ui, !waiting, crate::i18n::tr("Review"))
                    };
                    control(widgets, "dialog-preview-refresh", &r, ui.clip_rect());
                    f.refresh |= r.clicked();
                    let status = if s.running() {
                        crate::i18n::tr("Checking the submodule")
                    } else if d.error.is_some() {
                        crate::i18n::tr("Fix the problem above, then review again")
                    } else if reviewed {
                        crate::i18n::tr("Reviewed · nothing has changed yet")
                    } else {
                        crate::i18n::tr("Nothing changes until you review and apply.")
                    };
                    pages::footer_status(ui, waiting, status);
                }
            },
        );
        // The × returns to the Submodules page; automation's old Cancel id is that ×.
        if let Some(close) = self.frame_widgets.get("dialog-close").copied() {
            self.frame_widgets.insert("dialog-submodule-back".into(), close);
        }
        f.back |= response.should_close() && !s.applying;
        let source = s.source.clone();
        self.finish_submodule_frame(d, f);
        if browse && self.dialog.is_some() {
            self.browse(crate::folder_browser::PickTarget::SubmoduleSource, &source);
        }
    }

    /// Apply one frame's page or review choices.
    fn finish_submodule_frame(&mut self, mut d: Dialog, mut f: Flags) {
        let s = d.submodules.get_or_insert_with(Default::default);
        if let Some(mode) = f.mode {
            if mode == ManagementMode::Move {
                s.new_path.clear();
            }
            if mode == ManagementMode::SetUrl {
                s.source = s.selected_entry().map(|e| e.url.clone()).unwrap_or_default();
            }
            s.mode = mode;
            s.preview = None;
            s.action_preview = None;
            s.notice = None;
            d.error = None;
            f.refresh = mode != ManagementMode::Move;
        }
        if f.back {
            s.reviewing = false;
            s.mode = ManagementMode::Browse;
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
            match ReadTask::spawn(move |cancel| backend.submodule_open(&path, &entry, &cancel)) {
                Ok(t) => s.open_task = Some(t),
                Err(e) => d.error = Some(user_error(&e)),
            }
        }
        let action = s
            .action_preview
            .clone()
            .map(Action::SubmoduleAction)
            .or_else(|| s.preview.clone().map(Action::Submodule));
        if f.submit
            && let (Some(owner), Some(action)) = (d.owner, action)
        {
            s.applying = true;
            self.execute(owner, action);
            if self.write.is_none() {
                s.applying = false;
                d.error = Some(
                    crate::i18n::tr("Could not start. Wait for the current refresh, then review again.").into(),
                );
            }
        }
        if !f.dismiss {
            self.dialog = Some(d);
            if f.refresh {
                self.start_submodule_read();
            }
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
    mode: Option<ManagementMode>,
}

fn state_label(entry: &SubmoduleEntry) -> &'static str {
    match entry.state() {
        "Needs attention" => crate::i18n::tr("Needs attention"),
        "Local changes" => crate::i18n::tr("Local changes"),
        "Not initialized" => crate::i18n::tr("Not initialized"),
        "Update available" => crate::i18n::tr("Update available"),
        "Up to date" => crate::i18n::tr("Up to date"),
        other => other,
    }
}

fn entry_details(ui: &mut Ui, e: &SubmoduleEntry) {
    ui.with_layout(Layout::top_down(Align::Min), |ui| {
        ui.label(RichText::new(e.path.display_lossy()).font(semibold(15.0)));
        pages::field(ui, crate::i18n::tr("Source"), &e.url.to_string(), true);
        pages::field(ui, crate::i18n::tr("Recorded"), &e.target.to_string(), true);
        pages::field(
            ui,
            crate::i18n::tr("Checked out"),
            &format!(
                "{}{}",
                e.head
                    .as_ref()
                    .map_or(crate::i18n::tr("Not initialized").into(), ToString::to_string),
                e.branch
                    .as_ref()
                    .map_or("".into(), |b| format!(" · {}", String::from_utf8_lossy(b)))
            ),
            true,
        );
        pages::field(ui, crate::i18n::tr("State"), state_label(e), false);
        if let Some(f) = e.blocked {
            pages::callout(ui, AMBER(), f.guidance());
        }
    });
}

/// The review's title, one line on what it does, and whether it removes something.
fn review_heading(
    mode: ManagementMode,
    e: Option<&SubmoduleEntry>,
    repository: &str,
) -> (String, String, bool) {
    let path = e.map_or_else(|| crate::i18n::tr("the submodule").to_owned(), |e| e.path.display_lossy().to_string());
    match mode {
        ManagementMode::Move => (
            crate::i18n::tr("Move / rename submodule").into(),
            crate::i18n::trf!("Move {path} and stage its new path. Recovery metadata is kept.", path),
            false,
        ),
        ManagementMode::SetUrl => (
            crate::i18n::tr("Edit submodule URL").into(),
            crate::i18n::trf!("Change where {path} is fetched from. Its files and commits are kept.", path),
            false,
        ),
        ManagementMode::Sync => (
            crate::i18n::tr("Sync submodule URL").into(),
            crate::i18n::tr("Copy the URL in .gitmodules to this repository's settings.").into(),
            false,
        ),
        ManagementMode::Add => (
            crate::i18n::tr("Add submodule").into(),
            match repository {
                "" => crate::i18n::tr("Add a repository as a submodule.").into(),
                name => crate::i18n::trf!("Add a repository as a submodule of {name}.", name),
            },
            false,
        ),
        ManagementMode::Remove => (
            crate::i18n::tr("Remove this submodule?").into(),
            crate::i18n::trf!("Remove {path} from this repository. Its checkout is kept for recovery.", path),
            true,
        ),
        ManagementMode::Deinitialize => (
            crate::i18n::trf!("Deinitialize {path}?", path),
            crate::i18n::tr("Takes the submodule's files out of this working copy. You can initialize it again any time.")
                .into(),
            true,
        ),
        ManagementMode::Browse => match e {
            Some(e) if !e.initialized => (
                crate::i18n::tr("Initialize submodule").into(),
                crate::i18n::trf!("Check out {path} at the commit this repository records.", path),
                false,
            ),
            _ => (
                crate::i18n::tr("Update submodule").into(),
                crate::i18n::trf!("Check out the commit this repository records for {path}.", path),
                false,
            ),
        },
    }
}

fn short(id: &ObjectId) -> String {
    id.as_str().chars().take(8).collect()
}

/// A local repository's folder name.
fn folder_of(source: &str) -> String {
    let shown = crate::views::display_path(Path::new(source));
    Path::new(&shown)
        .file_name()
        .map_or_else(|| shown.clone(), |n| n.to_string_lossy().into_owned())
}

/// A source in a few words: a URL without its scheme, or a local repository's folder.
fn brief(source: &str) -> String {
    let shown = crate::views::display_path(Path::new(source));
    if Path::new(&shown).is_absolute() {
        return folder_of(&shown);
    }
    shown
        .split_once("://")
        .map_or(shown.as_str(), |(_, rest)| rest)
        .trim_end_matches('/')
        .to_owned()
}

/// A value for one line of `width`: a local path keeps its folder names nearest the end.
fn fitted(ui: &Ui, text: &str, width: f32) -> String {
    let shown = crate::views::display_path(Path::new(text));
    if Path::new(&shown).is_absolute() {
        crate::repository_forms::fit_path(ui, &shown, width)
    } else {
        shown
    }
}

fn plain(color: Color32) -> TextFormat {
    TextFormat::simple(FontId::proportional(13.0), color)
}

fn mono(color: Color32) -> TextFormat {
    TextFormat::simple(FontId::monospace(12.5), color)
}

/// One line of text, cut to the width left, with the whole text on hover.
fn line(ui: &mut Ui, mut job: text::LayoutJob, hover: &str) {
    job.wrap = text::TextWrapping::truncate_at_width(ui.available_width().max(20.0));
    ui.label(job).on_hover_text(hover);
}

/// A key on the left of a card row, its value to the right.
fn key_value(ui: &mut Ui, key: &str, width: f32, value: impl FnOnce(&mut Ui)) {
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 12.0;
        let (rect, _) = ui.allocate_exact_size(vec2(width, 18.0), Sense::hover());
        let mut job = text::LayoutJob::simple_singleline(key.to_owned(), FontId::proportional(13.0), HINT());
        job.wrap = text::TextWrapping::truncate_at_width(width);
        let galley = ui.painter().layout_job(job);
        ui.painter()
            .galley(pos2(rect.left(), rect.center().y - galley.size().y / 2.0), galley, HINT());
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            value(ui)
        });
    });
}

/// Where a submodule fetches from: a local repository's folder name after its parent,
/// faded and shortened from the left; a URL as written.
fn source_label(ui: &mut Ui, source: &str) {
    let shown = crate::views::display_path(Path::new(source));
    let text = fitted(ui, &shown, ui.available_width());
    let mut job = text::LayoutJob::default();
    match text.rfind(['\\', '/']).filter(|_| Path::new(&shown).is_absolute()) {
        Some(i) => {
            job.append(&text[..=i], 0.0, plain(FAINT()));
            job.append(&text[i + 1..], 0.0, plain(TEXT()));
        }
        None => job.append(&text, 0.0, plain(TEXT())),
    }
    line(ui, job, &shown);
}

/// The submodule a review acts on: its path and state, where it fetches from, the
/// commit this repository records and what is checked out.
fn module_card(ui: &mut Ui, e: &SubmoduleEntry) {
    let color = if e.blocked.is_some() || e.dirty {
        pages::WARN()
    } else if e.initialized {
        pages::OK()
    } else {
        pages::NEUTRAL()
    };
    pages::titled_card(
        ui,
        |ui| {
            let (rect, _) = ui.allocate_exact_size(vec2(15.0, 15.0), Sense::hover());
            icon(ui.painter(), rect, Icon::Modules, BLUE());
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                pages::pill(ui, state_label(e), color);
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    ui.add(
                        Label::new(RichText::new(e.path.display_lossy().to_string()).font(semibold(13.0)))
                            .truncate(),
                    );
                });
            });
        },
        |ui| {
            key_value(ui, crate::i18n::tr("Fetches from"), 112.0, |ui| source_label(ui, &e.url));
            key_value(ui, crate::i18n::tr("Recorded"), 112.0, |ui| {
                let mut job = text::LayoutJob::default();
                job.append(&short(&e.target), 0.0, mono(TEXT()));
                job.append(crate::i18n::tr(" · pinned by this repository"), 0.0, plain(FAINT()));
                line(ui, job, e.target.as_str());
            });
            key_value(ui, crate::i18n::tr("Checked out"), 112.0, |ui| match &e.head {
                Some(head) => {
                    let mut job = text::LayoutJob::default();
                    job.append(&short(head), 0.0, mono(TEXT()));
                    match &e.branch {
                        Some(branch) => {
                            job.append(crate::i18n::tr(" on "), 0.0, plain(FAINT()));
                            let tag = TextFormat {
                                background: CHIP(),
                                ..mono(TEXT_SOFT())
                            };
                            job.append(&format!(" {} ", String::from_utf8_lossy(branch)), 0.0, tag);
                        }
                        None => job.append(crate::i18n::tr(" · detached"), 0.0, plain(FAINT())),
                    }
                    line(ui, job, head.as_str());
                }
                None => {
                    ui.label(RichText::new(crate::i18n::tr("Not initialized")).color(HINT()));
                }
            });
        },
    );
    if let Some(failure) = e.blocked {
        pages::callout(ui, AMBER(), failure.guidance());
    }
}

fn field_name(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).size(12.0).color(MUTED()));
}

/// One status line under a field; an icon when it says something was checked.
fn status_line(ui: &mut Ui, image: Option<Icon>, color: Color32, text: &str) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 5.0;
        if let Some(image) = image {
            let (rect, _) = ui.allocate_exact_size(vec2(13.0, 13.0), Sense::hover());
            icon(ui.painter(), rect, image, color);
        }
        ui.add(Label::new(RichText::new(text).size(12.0).color(color)).truncate());
    });
}

/// One option of a two-way choice; the chosen one is filled.
fn segment(ui: &mut Ui, label: &str, selected: bool, enabled: bool) -> Response {
    let ink = if selected { TEXT() } else { MUTED() };
    ui.add_enabled(
        enabled,
        Button::new(RichText::new(label).size(12.5).color(ink))
            .selected(selected)
            .fill(if selected { SELECTED() } else { Color32::TRANSPARENT })
            .stroke(Stroke::NONE)
            .corner_radius(5)
            .min_size(vec2(0.0, 24.0)),
    )
}

/// The source field with Browse, and what the review found about it. Returns whether
/// the field was edited.
fn source_input(
    ui: &mut Ui,
    s: &mut DialogState,
    widgets: &mut HashMap<String, UiControl>,
    label: &str,
    browse: &mut bool,
) -> bool {
    let mut edited = false;
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 6.0;
        field_name(ui, label);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            // As wide as its label needs: Browse is a longer word in some languages.
            let button = crate::pages::button_width(ui, crate::i18n::tr("Browse"), 76.0);
            let width = ui.available_width() - button - 8.0;
            let source = &mut s.source;
            let r = ui
                .add_enabled_ui(!s.applying, |ui| {
                    ui.add_sized(
                        [width, CONTROL],
                        TextEdit::singleline(source)
                            .font(TextStyle::Monospace)
                            .align(Align2::LEFT_CENTER),
                    )
                })
                .inner;
            control(widgets, "dialog-submodule-source", &r, ui.clip_rect());
            edited = r.changed();
            let b = ui
                .add_enabled_ui(!s.applying, |ui| ui.add_sized([button, CONTROL], Button::new(crate::i18n::tr("Browse"))))
                .inner;
            control(widgets, "dialog-submodule-source-browse", &b, ui.clip_rect());
            *browse |= b.clicked();
        });
        match s.action_preview.as_ref().filter(|_| !edited) {
            Some(p) => {
                let (found, what) = source_found(p);
                if found {
                    status_line(ui, Some(Icon::Check), GREEN(), &what);
                } else {
                    status_line(ui, None, HINT(), &what);
                }
            }
            None => status_line(ui, None, HINT(), crate::i18n::tr("Checked when you review")),
        }
    });
    edited
}

/// What a review found about the source, and whether it vouches for it. Only Add reads
/// the source (ls-remote, for the commit it pins); a URL change is not contacted until
/// the next fetch. Other previews carry the recorded commit as their target, which is not
/// a sign that the source was read.
fn source_found(p: &SubmoduleActionPreview) -> (bool, String) {
    if matches!(p.action(), SubmoduleAction::Add { .. }) {
        (true, crate::i18n::trf!("Repository found · {source}", source = brief(p.source())))
    } else {
        (false, crate::i18n::trf!("Used from the next fetch · {source}", source = brief(p.source())))
    }
}

/// Add: where the new submodule goes, its source and its folder.
fn add_inputs(
    ui: &mut Ui,
    s: &mut DialogState,
    error: &mut Option<String>,
    widgets: &mut HashMap<String, UiControl>,
    waiting: bool,
    browse: &mut bool,
) {
    let mut edited = false;
    let child = s
        .selected_entry()
        .filter(|e| e.initialized)
        .map(|e| (e.path.display_lossy().to_string(), e.directory.clone()));
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 6.0;
        field_name(ui, crate::i18n::tr("Add to"));
        Frame::new()
            .fill(RAISED())
            .stroke(Stroke::new(1.0, HAIRLINE()))
            .corner_radius(7)
            .inner_margin(2)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    let r = segment(ui, crate::i18n::tr("This repository"), s.add_parent.is_none(), !waiting);
                    control(widgets, "dialog-submodule-parent-current", &r, ui.clip_rect());
                    if r.clicked() {
                        s.add_parent = None;
                        edited = true;
                    }
                    let label = child
                        .as_ref()
                        .map_or_else(|| crate::i18n::tr("Inside a submodule").to_owned(), |(path, _)| crate::i18n::trf!("Inside {path}", path));
                    let r = segment(ui, &label, s.add_parent.is_some(), !waiting && child.is_some());
                    control(widgets, "dialog-submodule-parent-selected", &r, ui.clip_rect());
                    if r.clicked() {
                        s.add_parent = child.as_ref().map(|(_, directory)| directory.clone());
                        edited = true;
                    }
                });
            });
    });
    edited |= source_input(ui, s, widgets, crate::i18n::tr("Source URL or local repository"), browse);
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 6.0;
        field_name(ui, crate::i18n::tr("Folder in this repository"));
        let width = ui.available_width().min(360.0);
        let path = &mut s.new_path;
        let r = ui
            .add_enabled_ui(!s.applying, |ui| {
                ui.add_sized(
                    [width, CONTROL],
                    TextEdit::singleline(path)
                        .font(TextStyle::Monospace)
                        .align(Align2::LEFT_CENTER)
                        .hint_text("vendor/library"),
                )
            })
            .inner;
        control(widgets, "dialog-submodule-path", &r, ui.clip_rect());
        edited |= r.changed();
        if s.action_preview.is_some() && !edited {
            status_line(ui, Some(Icon::Check), GREEN(), crate::i18n::tr("New folder"));
        } else {
            status_line(ui, None, HINT(), crate::i18n::tr("A folder that does not exist yet"));
        }
    });
    if edited {
        s.action_task = None;
        s.action_preview = None;
        *error = None;
    }
}

/// One reviewed change: what it touches, then the old value struck through and the new.
struct Change {
    what: &'static str,
    old: Option<String>,
    new: String,
}

impl Change {
    fn new(what: &'static str, old: Option<String>, new: impl Into<String>) -> Self {
        Self {
            what,
            old,
            new: new.into(),
        }
    }
}

fn change_row(ui: &mut Ui, c: &Change) {
    key_value(ui, c.what, 150.0, |ui| {
        let width = ui.available_width();
        match &c.old {
            Some(old) if *old == c.new => {
                let mut job = text::LayoutJob::default();
                job.append(&fitted(ui, &c.new, width - 80.0), 0.0, plain(TEXT()));
                job.append(" (unchanged)", 0.0, TextFormat::simple(FontId::proportional(12.5), HINT()));
                line(ui, job, &c.new);
            }
            Some(old) => {
                let mut job = text::LayoutJob::default();
                let struck = TextFormat {
                    strikethrough: Stroke::new(1.0, FAINT()),
                    ..plain(FAINT())
                };
                job.append(&fitted(ui, old, width), 0.0, struck);
                line(ui, job, old);
                let mut job = text::LayoutJob::default();
                job.append(&format!("→ {}", fitted(ui, &c.new, width - 20.0)), 0.0, plain(GREEN_TEXT()));
                line(ui, job, &c.new);
            }
            None => {
                let mut job = text::LayoutJob::default();
                job.append(&fitted(ui, &c.new, width), 0.0, plain(TEXT()));
                line(ui, job, &c.new);
            }
        }
    });
}

/// What a review found: a header line, each change, and what to do next.
fn review_card(ui: &mut Ui, title: &str, meta: &str, danger: bool, changes: &[Change], note: &str) {
    pages::titled_card(
        ui,
        |ui| {
            let color = if danger { RED() } else { GREEN() };
            let (mark, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
            ui.painter()
                .circle_stroke(mark.center(), 6.0, Stroke::new(1.4, color));
            icon(ui.painter(), mark.shrink(3.5), if danger { Icon::Close } else { Icon::Check }, color);
            ui.label(RichText::new(title).font(semibold(13.0)));
            if !meta.is_empty() {
                ui.add(Label::new(RichText::new(meta).size(12.0).color(HINT())).truncate());
            }
        },
        |ui| {
            for (i, c) in changes.iter().enumerate() {
                if i > 0 {
                    pages::divider(ui);
                }
                change_row(ui, c);
            }
            if !note.is_empty() {
                pages::divider(ui);
                ui.add(Label::new(RichText::new(note).size(12.5).color(MUTED())).wrap());
            }
        },
    );
}

/// The URLs a sync or URL edit changes: this repository's, then the child's.
fn url_changes(changes: &mut Vec<Change>, previous: &[String], new: &str) {
    for (i, old) in previous.iter().enumerate() {
        let what = if i == 0 {
            crate::i18n::tr("This repository's config")
        } else {
            crate::i18n::tr("Fetch URL in the child")
        };
        changes.push(Change::new(what, Some(old.clone()), new));
    }
}

fn descendants_note(n: usize) -> String {
    match n {
        0 => String::new(),
        n => crate::i18n::trf!("Initialized nested submodules moving with the checkout: {count}.", count = n),
    }
}

/// What an add, removal, deinitialization or URL change will do.
fn action_review(ui: &mut Ui, p: &SubmoduleActionPreview, entry: Option<&SubmoduleEntry>) {
    let path = p.path().display_lossy().to_string();
    let recorded = entry.map_or_else(String::new, |e| crate::i18n::trf!("recorded {commit}", commit = short(&e.target)));
    let mut changes = Vec::new();
    let (title, meta, note, danger) = match p.action() {
        SubmoduleAction::Move { destination, .. } => {
            changes.push(Change::new(crate::i18n::tr("Checkout path"), Some(path.clone()), destination.display_lossy()));
            changes.push(Change::new(crate::i18n::tr("Gitlink and .gitmodules"), None, crate::i18n::tr("New path staged")));
            changes.push(Change::new(crate::i18n::tr("Nested checkouts"), None, crate::i18n::trf!("Initialized descendants moving with the checkout: {count}", count = p.descendants())));
            (crate::i18n::trf!("Moves {path}", path), recorded, crate::i18n::tr("Recovery metadata is saved first. Stop cancels preparation; once the folder moves, Rocket finishes the prepared link updates. Commit to record the new path.").into(), false)
        }
        SubmoduleAction::SetUrl { .. } => {
            changes.push(Change::new(crate::i18n::tr(".gitmodules (staged)"), None, p.source()));
            url_changes(&mut changes, p.previous_urls(), p.source());
            (
                crate::i18n::trf!("Sets the URL of {path}", path),
                String::new(),
                crate::i18n::tr("Commit .gitmodules to share the new URL.").to_owned(),
                false,
            )
        }
        SubmoduleAction::Sync(_) => {
            url_changes(&mut changes, p.previous_urls(), p.source());
            let title = if changes.iter().all(|c| c.old.as_deref() == Some(c.new.as_str())) {
                crate::i18n::tr("Local settings already match .gitmodules")
            } else {
                crate::i18n::tr("Local settings will match .gitmodules")
            };
            (
                title.to_owned(),
                path.clone(),
                crate::i18n::tr("Nothing is staged; only local settings change.").to_owned(),
                false,
            )
        }
        SubmoduleAction::Add { .. } => {
            changes.push(Change::new(crate::i18n::tr("Parent repository"), None, folder_of(&p.parent().display().to_string())));
            if let Some(id) = p.target() {
                changes.push(Change::new(crate::i18n::tr("Pinned commit"), None, crate::i18n::trf!("{commit} · detached checkout", commit = short(id))));
            }
            changes.push(Change::new(crate::i18n::tr("Gitlink and .gitmodules"), None, crate::i18n::tr("New entry, staged")));
            (
                crate::i18n::trf!("Adds {path}", path),
                crate::i18n::trf!("from {source}", source = brief(p.source())),
                crate::i18n::tr("Commit to record the submodule.").to_owned(),
                false,
            )
        }
        SubmoduleAction::Remove(_) => {
            changes.push(Change::new(crate::i18n::tr("Gitlink and .gitmodules"), None, crate::i18n::tr("Removal staged")));
            changes.push(Change::new(crate::i18n::tr("Checkout"), None, crate::i18n::tr("Moved to recovery storage")));
            (crate::i18n::trf!("Removes {path}", path), recorded, descendants_note(p.descendants()), true)
        }
        SubmoduleAction::Deinitialize(_) => {
            let mut note = descendants_note(p.descendants());
            if !note.is_empty() {
                note.push(' ');
            }
            note.push_str("Nothing is staged and no commit changes. Other clones aren't affected.");
            effects_card(
                ui,
                &[
                    (AMBER(), Icon::External, crate::i18n::tr("Files moved aside"), crate::i18n::tr("kept in recovery storage, not deleted")),
                    (RED(), Icon::Close, crate::i18n::tr("Local registration removed"), crate::i18n::tr("from this repository's config")),
                    (GREEN(), Icon::Check, crate::i18n::tr("Recorded commit and .gitmodules unchanged"), crate::i18n::tr("still tracked by this repository")),
                ],
                &note,
            );
            return;
        }
    };
    review_card(ui, &title, &meta, danger, &changes, &note);
}

/// "What changes": one marked line per effect, then a note.
fn effects_card(ui: &mut Ui, effects: &[(Color32, Icon, &str, &str)], note: &str) {
    pages::titled_card(
        ui,
        |ui| {
            ui.label(RichText::new(crate::i18n::tr("What changes")).size(12.0).color(MUTED()).strong());
        },
        |ui| {
            for (i, &(color, image, what, detail)) in effects.iter().enumerate() {
                if i > 0 {
                    pages::divider(ui);
                }
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 10.0;
                    let (mark, _) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::hover());
                    ui.painter()
                        .circle_filled(mark.center(), 11.0, color.gamma_multiply(0.14));
                    icon(ui.painter(), mark.shrink(6.0), image, color);
                    let mut job = text::LayoutJob::default();
                    job.append(what, 0.0, TextFormat::simple(semibold(13.0), TEXT()));
                    job.append(
                        &format!(" · {detail}"),
                        0.0,
                        TextFormat::simple(FontId::proportional(13.0), HINT()),
                    );
                    job.wrap.max_width = ui.available_width();
                    ui.label(job);
                });
            }
            pages::divider(ui);
            ui.add(Label::new(RichText::new(note).size(12.5).color(MUTED())).wrap());
        },
    );
}

/// What updating or initializing the checkout will do: its title and changes.
fn checkout_changes(p: &SubmodulePreview) -> (String, [Change; 2]) {
    let e = &p.entry;
    let target = short(&e.target);
    let new = crate::i18n::trf!("{target} (detached)", target);
    let (title, old) = if !e.initialized {
        (crate::i18n::trf!("Initializes at {target}", target), None)
    } else if e.head.as_ref() == Some(&e.target) {
        // Shown as unchanged rather than as itself struck through.
        (crate::i18n::trf!("Already at {target}", target), Some(new.clone()))
    } else {
        let head = e.head.as_ref().map_or_else(|| crate::i18n::tr("None").to_owned(), short);
        (crate::i18n::trf!("Checks out {target}", target), Some(head))
    };
    let changes = [
        Change::new(crate::i18n::tr("Checkout"), old, new),
        Change::new(crate::i18n::tr("Source"), None, p.source.as_str()),
    ];
    (title, changes)
}

fn checkout_review(ui: &mut Ui, p: &SubmodulePreview) {
    let (title, changes) = checkout_changes(p);
    review_card(ui, &title, crate::i18n::tr("detached"), false, &changes, "");
}

/// A problem or warning in plain words, with Rocket's code as small print.
fn banner(ui: &mut Ui, color: Color32, message: &str, code: Option<&str>) {
    Frame::new()
        .fill(color.gamma_multiply(0.08))
        .stroke(Stroke::new(1.0, color.gamma_multiply(0.3)))
        .corner_radius(8)
        .inner_margin(Margin::symmetric(12, 9))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 3.0;
            ui.add(Label::new(RichText::new(message).size(12.5).color(color)).wrap());
            if let Some(code) = code {
                ui.label(RichText::new(code).monospace().size(11.0).color(FAINT()));
            }
        });
}

/// A submodule error's code and its guidance (`Submodule [code]: what to do`).
fn split_error(error: &str) -> (Option<&str>, &str) {
    if let Some(rest) = error.strip_prefix("Submodule [")
        && let Some((code, message)) = rest.split_once("]: ")
    {
        return (Some(code), message);
    }
    (None, error.strip_prefix("invalid input: ").unwrap_or(error))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_common::{self as common, Fixture};
    use std::{
        thread,
        time::{Duration, Instant},
    };

    #[test]
    fn switching_repository_dismisses_submodule_preview_without_writing() {
        common::isolated(
            "ROCKET_SUBMODULE_UI_CASE",
            "submodules::tests::switching_repository_dismisses_submodule_preview_without_writing",
            || {
                let first = Fixture::new();
                first.write("base", "parent");
                first.commit("parent");
                let source = Fixture::new();
                source.write("base", "child");
                source.commit("child");
                let target = String::from_utf8(source.run(&["rev-parse", "HEAD"])).unwrap();
                first.run(&[
                    "config",
                    "--file",
                    ".gitmodules",
                    "submodule.child.path",
                    "child 文",
                ]);
                first.run(&[
                    "config",
                    "--file",
                    ".gitmodules",
                    "submodule.child.url",
                    source.repository.to_str().unwrap(),
                ]);
                first.run(&["add", ".gitmodules"]);
                first.run(&[
                    "update-index",
                    "--add",
                    "--cacheinfo",
                    &format!("160000,{},child 文", target.trim()),
                ]);
                let ctx = Context::default();
                let mut app = Rocket::with_log_directory(
                    &ctx,
                    vec![first.repository.clone(), source.repository.clone()],
                    first.root.join("logs"),
                );
                let wait = |app: &mut Rocket| {
                    let deadline = Instant::now() + Duration::from_secs(45);
                    while app.busy() {
                        app.poll(&ctx);
                        assert!(Instant::now() < deadline);
                        thread::sleep(Duration::from_millis(10));
                    }
                };
                wait(&mut app);
                app.auto_refresh = false;
                app.active = 0;
                app.dispatch(Intent::Dialog(DialogKind::Submodules));
                wait(&mut app);
                let s = app.dialog.as_mut().unwrap().submodules.as_mut().unwrap();
                s.selected = Some(s.entries[0].path.clone());
                s.reviewing = true;
                app.start_submodule_read();
                wait(&mut app);
                assert!(
                    app.dialog
                        .as_ref()
                        .unwrap()
                        .submodules
                        .as_ref()
                        .unwrap()
                        .preview
                        .is_some()
                );
                app.active = 1;
                let mut out = ctx.run_ui(Default::default(), |_| app.submodule_dialog(&ctx));
                out.textures_delta.clear();
                assert!(app.dialog.is_none());
                assert!(app.write.is_none());
                assert!(!first.repository.join("child 文").exists());
            },
        );
    }

    #[test]
    fn errors_and_sources_read_in_a_few_words() {
        assert_eq!(
            split_error("Submodule [submodule_dirty]: Commit or stash the child changes first."),
            (Some("submodule_dirty"), "Commit or stash the child changes first.")
        );
        assert_eq!(split_error("invalid input: Select a submodule to sync"), (None, "Select a submodule to sync"));
        assert_eq!(brief("https://example.com/team/new-lib.git"), "example.com/team/new-lib.git");
        let local = if cfg!(windows) { r"\\?\C:\work\PRIVATE_child_文" } else { "/work/PRIVATE_child_文" };
        assert_eq!(brief(local), "PRIVATE_child_文");
        let removal = review_heading(ManagementMode::Remove, None, "parent");
        assert!(removal.2 && removal.0 == "Remove this submodule?");
        assert_eq!(review_heading(ManagementMode::Add, None, "parent").1, "Add a repository as a submodule of parent.");
    }

    #[test]
    fn reviews_share_one_layout_and_apply_only_after_a_clean_review() {
        common::isolated(
            "ROCKET_SUBMODULE_UI_CASE",
            "submodules::tests::reviews_share_one_layout_and_apply_only_after_a_clean_review",
            || {
                use crate::batch::flow_tests::{click, enabled, frame, open, wait};
                let first = Fixture::new();
                first.write("base", "parent");
                first.commit("parent");
                let source = Fixture::new();
                source.write("base", "child");
                source.commit("child");
                let target = String::from_utf8(source.run(&["rev-parse", "HEAD"])).unwrap();
                first.run(&["config", "--file", ".gitmodules", "submodule.child.path", "child 文"]);
                first.run(&[
                    "config",
                    "--file",
                    ".gitmodules",
                    "submodule.child.url",
                    source.repository.to_str().unwrap(),
                ]);
                first.run(&["add", ".gitmodules"]);
                first.run(&["update-index", "--add", "--cacheinfo", &format!("160000,{},child 文", target.trim())]);
                // Commit only what is staged: the child folder does not exist yet.
                first.run(&["commit", "-m", "register child"]);
                let ctx = Context::default();
                let mut app = open(&ctx, &first);
                app.dispatch(Intent::Dialog(DialogKind::Submodules));
                wait(&ctx, &mut app);
                click(&ctx, &mut app, "dialog-submodule-row-0");
                let shown =|app: &Rocket, key: &str| app.frame_widgets.contains_key(key);
                // Sync needs the child registered in this repository: the review fails, says
                // why, and offers Review again; the operation stays disabled.
                click(&ctx, &mut app, "dialog-submodule-sync");
                wait(&ctx, &mut app);
                let error = app.dialog.as_ref().unwrap().error.clone().unwrap_or_default();
                assert!(error.contains("submodule_"), "{error}");
                assert!(!enabled(&ctx, &mut app, "dialog-submit"));
                assert!(enabled(&ctx, &mut app, "dialog-preview-refresh"));
                // The × returns to the page, which reads the list again.
                click(&ctx, &mut app, "dialog-cancel");
                wait(&ctx, &mut app);
                // Add: nothing is reviewed yet, so there is no operation button; inside a
                // submodule needs an initialized one.
                click(&ctx, &mut app, "dialog-submodule-add");
                frame(&ctx, &mut app, vec![]);
                assert!(!shown(&app, "dialog-submit"));
                assert!(enabled(&ctx, &mut app, "dialog-submodule-source-browse"));
                assert!(enabled(&ctx, &mut app, "dialog-submodule-parent-current"));
                assert!(!enabled(&ctx, &mut app, "dialog-submodule-parent-selected"));
                click(&ctx, &mut app, "dialog-cancel");
                wait(&ctx, &mut app);
                // Initialization: reviewed, then the operation by name can run.
                click(&ctx, &mut app, "dialog-submodule-row-0");
                click(&ctx, &mut app, "dialog-submodule-review");
                wait(&ctx, &mut app);
                assert!(app.dialog.as_ref().unwrap().submodules.as_ref().unwrap().preview.is_some());
                assert!(enabled(&ctx, &mut app, "dialog-submit"));
                click(&ctx, &mut app, "dialog-cancel");
                assert!(!first.repository.join("child 文").join("base").exists(), "nothing was checked out");
            },
        );
    }

    #[test]
    fn reviews_claim_only_what_they_checked() {
        common::isolated(
            "ROCKET_SUBMODULE_UI_CASE",
            "submodules::tests::reviews_claim_only_what_they_checked",
            || {
                use crate::batch::flow_tests::{open, wait};
                let parent = Fixture::new();
                parent.write("base", "parent");
                parent.commit("parent");
                let source = Fixture::new();
                source.write("base", "child");
                source.commit("child");
                let url = source.repository.to_str().unwrap();
                parent.run(&["-c", "protocol.file.allow=always", "submodule", "add", url, "child"]);
                parent.run(&["commit", "-m", "add child"]);
                let ctx = Context::default();
                let mut app = open(&ctx, &parent);
                app.dispatch(Intent::Dialog(DialogKind::Submodules));
                wait(&ctx, &mut app);
                let s = app.dialog.as_mut().unwrap().submodules.as_mut().unwrap();
                s.selected = Some(s.entries[0].path.clone());
                s.reviewing = true;
                app.start_submodule_read();
                wait(&ctx, &mut app);
                let d = app.dialog.as_ref().unwrap();
                let preview = d.submodules.as_ref().unwrap().preview.clone();
                let preview = preview.unwrap_or_else(|| panic!("no review: {:?}", d.error));
                let (title, changes) = checkout_changes(&preview);
                assert!(title.starts_with("Already at"), "{title}");
                assert_eq!(changes[0].old.as_deref(), Some(changes[0].new.as_str()));
                // A URL edit is not contacted by its review, so even a folder that is no
                // repository must not read as found.
                let folder = parent.root.join("not a repository");
                std::fs::create_dir(&folder).unwrap();
                let s = app.dialog.as_mut().unwrap().submodules.as_mut().unwrap();
                s.reviewing = false;
                s.mode = ManagementMode::SetUrl;
                s.source = folder.to_string_lossy().into_owned();
                app.start_submodule_read();
                wait(&ctx, &mut app);
                let d = app.dialog.as_ref().unwrap();
                let review = d.submodules.as_ref().unwrap().action_preview.clone();
                let review = review.unwrap_or_else(|| panic!("no review: {:?}", d.error));
                let (found, text) = source_found(&review);
                assert!(!found && text.starts_with("Used from the next fetch"), "{text}");
            },
        );
    }
}
