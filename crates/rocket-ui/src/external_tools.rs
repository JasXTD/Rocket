//! External tools share the existing checked conflict editor publication path.
use crate::{
    app::*,
    backend::user_error,
    folder_browser::PickTarget,
    known_tools::{self, KnownTool},
    preferences::status_line,
    repository_forms::{Form, Status},
    style::*,
    terminal_settings::check_program,
};
use eframe::egui::*;
use rocket_git::*;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

pub struct State {
    pub open: bool,
    pub kind: ExternalToolKind,
    pub profiles: crate::tool_profiles::Profiles,
    pub editing: crate::tool_profiles::Profiles,
    pub selected: [usize; 3],
    pub removing: bool,
    pub owner: Option<u64>,
    pub label: String,
    pub error: Option<String>,
    pub notice: Option<String>,
    pub report: Option<ExternalToolReport>,
    pub imported: bool,
    pending: Option<ExternalToolRequest>,
    task: Option<WriteTask<ExternalToolReport>>,
    import: Option<ReadTask<()>>,
    draft: String,
    pub cancelling: bool,
    /// What the open Add list found on this computer.
    known: Option<Vec<(&'static KnownTool, Option<PathBuf>)>>,
    /// Whether the selected profile's program is there.
    program_check: Form,
}
impl Default for State {
    fn default() -> Self {
        let profiles = crate::tool_profiles::Profiles::default();
        Self {
            open: false,
            kind: ExternalToolKind::Editor,
            editing: profiles.clone(),
            profiles,
            selected: [0; 3],
            removing: false,
            owner: None,
            label: String::new(),
            error: None,
            notice: None,
            report: None,
            imported: false,
            pending: None,
            task: None,
            import: None,
            draft: String::new(),
            cancelling: false,
            known: None,
            program_check: Default::default(),
        }
    }
}
impl State {
    pub fn running(&self) -> bool {
        self.task.is_some() || self.import.is_some()
    }
    pub fn pending(&self) -> bool {
        self.pending.is_some()
    }
    /// The merge result is being checked before it opens in the conflict editor.
    pub fn importing(&self) -> bool {
        self.import.is_some()
    }
    /// Forget a launch that was prepared but not started.
    pub fn clear_pending(&mut self) {
        self.pending = None;
    }
    pub fn cancel(&mut self) {
        if let Some(t) = &self.task {
            t.cancel();
        }
        if let Some(t) = &self.import {
            t.cancel();
        }
        self.cancelling = self.running();
    }
    pub fn config(&self) -> &ExternalToolConfig {
        let group = &self.profiles.0[self.kind.index()];
        &group
            .entries
            .get(self.selected[self.kind.index()])
            .unwrap_or(&group.entries[group.default])
            .config
    }
    /// Preferences → External tools opened: its program checks start afresh.
    pub(crate) fn page_opened(&mut self) {
        self.program_check = Default::default();
    }
    pub fn reset_edits(&mut self) {
        self.editing = self.profiles.clone();
        self.selected = self.profiles.selections();
        self.removing = false;
    }
}
#[derive(Clone, Copy)]
pub enum ToolIntent {
    Prepare(ExternalToolKind),
    Start,
    Import,
    Cancel,
    Back,
    SaveSettings,
}

impl Rocket {
    pub(crate) fn external_intent(&mut self, intent: ToolIntent) {
        match intent {
            ToolIntent::Back => self.external.open = false,
            ToolIntent::Cancel => {
                if let Some(b) = &self.backend {
                    let _: rocket_git::Result<()> =
                        b.log.request("external-cancel", |_| Err(Error::Cancelled));
                }
                if self.external.running() {
                    self.external.cancel();
                } else {
                    self.external.pending = None;
                    self.external.imported = true;
                    self.external.notice = Some(
                        if self.external.kind == ExternalToolKind::Editor {
                            "Session cancelled. Editor saves already made remain in the working tree."
                        } else {
                            "Cancelled. No import or repository write. Temporary copies are retained."
                        }
                        .into(),
                    );
                }
            }
            ToolIntent::SaveSettings => {
                if self.external.running() {
                    return;
                }
                let validate = || self.external.editing.validate();
                let result = if let Some(backend) = &self.backend {
                    backend.log.request("external-settings", |_| validate())
                } else {
                    validate()
                };
                match result {
                    Ok(()) => {
                        self.external.profiles = self.external.editing.clone();
                        self.external.removing = false;
                        self.external.error = None;
                        self.external.notice = Some("Tool profiles updated.".into());
                    }
                    Err(e) => self.external.error = Some(user_error(&e)),
                }
            }
            ToolIntent::Prepare(kind) => {
                if self.external.running() || self.write.is_some() || self.quitting {
                    return;
                }
                let result = self.external_request(kind);
                self.external.open = true;
                match result {
                    Ok((owner, label, request, draft)) => {
                        self.external.kind = kind;
                        self.external.owner = Some(owner);
                        self.external.label = label;
                        self.external.pending = Some(request);
                        self.external.draft = draft;
                        self.external.report = None;
                        self.external.error = None;
                        self.external.notice = None;
                        self.external.imported = false;
                        self.external.reset_edits();
                    }
                    Err(e) => self.external.error = Some(user_error(&e)),
                }
            }
            ToolIntent::Start => {
                if self.external.running() || self.write.is_some() || self.quitting {
                    return;
                }
                if self.external.profiles != self.external.editing {
                    self.external.error =
                        Some("Save or discard profile edits before launching.".into());
                    return;
                }
                let Some(backend) = self.backend.clone() else {
                    return;
                };
                let Some(repo) = self
                    .tabs
                    .iter()
                    .find(|t| Some(t.id) == self.external.owner)
                    .and_then(|t| t.snapshot.as_ref())
                    .map(|s| s.repository.clone())
                else {
                    self.external.error =
                        Some("The source repository was closed. Open a fresh tool preview.".into());
                    return;
                };
                let Some(request) = self.external.pending.take() else {
                    return;
                };
                let config = self.external.config().clone();
                let storage = self.state_directory.join("external-tools");
                self.external.error = None;
                self.external.notice =
                    Some("Preparing copies and running the external tool…".into());
                self.external.cancelling = false;
                match WriteTask::spawn(move |cancel| {
                    Ok(repo.run_external_tool(&config, request, &storage, &backend.log, &cancel))
                }) {
                    Ok(task) => self.external.task = Some(task),
                    Err(e) => {
                        self.external.notice = None;
                        self.external.error = Some(user_error(&e));
                    }
                }
            }
            ToolIntent::Import => {
                if self.external.running() || self.write.is_some() || self.quitting {
                    return;
                }
                self.external.notice = None;
                if !self.external_binding_matches() {
                    self.reject_external_draft();
                    return;
                }
                let Some(report) = self
                    .external
                    .report
                    .as_ref()
                    .filter(|r| r.result.is_ok() && r.text.is_some())
                else {
                    return;
                };
                let Some(preview) = report.preview.clone() else {
                    return;
                };
                let Some(repo) = self
                    .tabs
                    .iter()
                    .find(|t| Some(t.id) == self.external.owner)
                    .and_then(|t| t.snapshot.as_ref())
                    .map(|s| s.repository.clone())
                else {
                    return;
                };
                let Some(backend) = self.backend.clone() else {
                    return;
                };
                match ReadTask::spawn(move |cancel| {
                    backend.log.request("external-import", |log| {
                        repo.with_log(log.clone())
                            .validate_conflict_preview(&preview, &cancel)
                    })
                }) {
                    Ok(task) => {
                        self.external.import = Some(task);
                        self.external.error = None;
                        self.external.notice =
                            Some("Checking the original file and conflict state…".into());
                    }
                    Err(e) => self.external.error = Some(user_error(&e)),
                }
            }
        }
    }
    fn external_request(
        &self,
        kind: ExternalToolKind,
    ) -> rocket_git::Result<(u64, String, ExternalToolRequest, String)> {
        let tab = self
            .tab()
            .ok_or_else(|| Error::InvalidInput("Open a repository first.".into()))?;
        let missing = || {
            Error::InvalidInput("Select a file in Changes or Commit → Files Changed first.".into())
        };
        let (label, request, draft) = match kind {
            ExternalToolKind::Merge => {
                let s = &tab.conflict;
                if s.running() {
                    return Err(Error::InvalidInput("Wait for the conflict preview.".into()));
                }
                let preview = s.preview.clone().ok_or_else(|| {
                    Error::InvalidInput(
                        "Open a text conflict in Rocket's conflict editor first.".into(),
                    )
                })?;
                (
                    preview.path().display_lossy().into_owned(),
                    ExternalToolRequest::Merge {
                        preview,
                        draft: s.draft.clone(),
                    },
                    s.draft.clone(),
                )
            }
            ExternalToolKind::Editor => {
                let path = if tab.view == View::Conflict {
                    tab.conflict.path.clone()
                } else if tab.view == View::Changes {
                    tab.selected_change.as_ref().map(|(p, _)| p.clone())
                } else {
                    None
                }
                .ok_or_else(missing)?;
                (
                    path.display_lossy().into_owned(),
                    ExternalToolRequest::Editor(path),
                    String::new(),
                )
            }
            ExternalToolKind::Diff => {
                if tab.view == View::Changes {
                    let (path, staged) = tab.selected_change.clone().ok_or_else(missing)?;
                    let untracked = tab
                        .snapshot
                        .as_ref()
                        .and_then(|s| s.status.as_ref())
                        .is_some_and(|s| {
                            s.entries
                                .iter()
                                .any(|e| e.path == path && e.kind == EntryKind::Untracked)
                        });
                    (
                        path.display_lossy().into_owned(),
                        ExternalToolRequest::Diff {
                            options: DiffOptions {
                                target: if staged {
                                    DiffTarget::Staged
                                } else {
                                    DiffTarget::Unstaged
                                },
                                ..Default::default()
                            },
                            path: Some(path),
                            label: None,
                            untracked,
                            comparison: None,
                        },
                        String::new(),
                    )
                } else if tab.view == View::Compare {
                    if tab.compare.running() {
                        return Err(Error::InvalidInput("Wait for the comparison to finish.".into()));
                    }
                    let loaded = tab.compare.loaded.as_ref().ok_or_else(missing)?;
                    let options = loaded.comparison.file_options(tab.compare.selected.ok_or_else(missing)?)?;
                    let file = tab.patch.as_ref().and_then(|p|p.files.get(tab.patch_file)).ok_or_else(missing)?;
                    (file.label.clone(), ExternalToolRequest::Diff {
                        options, path: None, label: Some(file.label.clone()), untracked: false, comparison: Some(std::sync::Arc::new(loaded.comparison.clone())),
                    }, String::new())
                } else if tab.view == View::Commits {
                    let id = tab.selected_commit.clone().ok_or_else(missing)?;
                    let file = tab
                        .patch
                        .as_ref()
                        .and_then(|p| p.files.get(tab.patch_file))
                        .ok_or_else(missing)?;
                    (
                        file.label.clone(),
                        ExternalToolRequest::Diff {
                            options: DiffOptions {
                                target: DiffTarget::Commit { id },
                                ..Default::default()
                            },
                            path: None,
                            label: Some(file.label.clone()),
                            untracked: false,
                            comparison: None,
                        },
                        String::new(),
                    )
                } else {
                    return Err(missing());
                }
            }
        };
        Ok((tab.id, label, request, draft))
    }
    fn reject_external_draft(&mut self) {
        let failure = Error::ExternalTool(ExternalToolFailure::DraftChanged);
        self.external.error = Some(user_error(&failure));
        if let Some(b) = &self.backend {
            let _: rocket_git::Result<()> = b.log.request("external-import", |_| Err(failure));
        }
    }
    fn external_binding_matches(&self) -> bool {
        let Some(preview) = self
            .external
            .report
            .as_ref()
            .and_then(|r| r.preview.as_ref())
        else {
            return false;
        };
        self.tabs
            .iter()
            .find(|t| Some(t.id) == self.external.owner)
            .is_some_and(|t| {
                t.conflict.path.as_ref() == Some(preview.path())
                    && !t.conflict.running()
                    && t.conflict.draft == self.external.draft
                    && t.conflict
                        .preview
                        .as_ref()
                        .is_some_and(|p| Arc::ptr_eq(p, preview))
            })
    }
    pub(crate) fn poll_external(&mut self, ctx: &Context) {
        if let Some(result) = self.external.task.as_mut().and_then(WriteTask::try_finish) {
            self.external.task = None;
            match result {
                Ok(mut report) => {
                    if self.external.cancelling && report.result.is_ok() {
                        report.result = Err(Error::Cancelled);
                    }
                    self.external.error = report.result.as_ref().err().map(user_error);
                    let notice = if report.result.is_ok() {
                        if report.text.is_some() {
                            "Tool exited successfully. Review the result before saving."
                        } else {
                            "Tool exited successfully. Repository changes will refresh automatically."
                        }
                    } else if self.external.kind == ExternalToolKind::Editor {
                        "Editor stopped or failed. Any saves it already made remain in the working tree."
                    } else {
                        "No merge result was imported. Any temporary workspace is retained."
                    };
                    self.external.notice = Some(notice.into());
                    self.external.report = Some(report);
                }
                Err(e) => {
                    self.external.notice = None;
                    self.external.error = Some(user_error(&e));
                }
            }
        }
        if self.external.task.is_none() && self.external.import.is_none() {
            self.external.cancelling = false;
        }
        if let Some(result) = self.external.import.as_mut().and_then(ReadTask::try_finish) {
            self.external.import = None;
            self.external.cancelling = false;
            self.external.notice = None;
            match result {
                Ok(()) if self.external_binding_matches() => {
                    let report = self.external.report.as_ref().unwrap();
                    let index = self
                        .tabs
                        .iter()
                        .position(|t| Some(t.id) == self.external.owner)
                        .unwrap();
                    self.tabs[index].conflict.import_external(
                        report.preview.clone().unwrap(),
                        report.text.clone().unwrap(),
                    );
                    self.tabs[index].view = View::Conflict;
                    self.active = index;
                    self.external.open = false;
                    self.external.imported = true;
                }
                // The user can change the draft while the file check is running.
                // Record this final refusal as well as the worker's file-check result.
                Ok(()) => self.reject_external_draft(),
                Err(e) => self.external.error = Some(user_error(&e)),
            }
        }
        if self.external.running() {
            ctx.request_repaint_after(Duration::from_millis(30));
        }
    }
    /// Preferences → External tools: the profiles of each tool type. Returns a Browse
    /// request for the selected profile's program.
    pub(crate) fn tools_page(&mut self, ui: &mut Ui) -> Option<PickTarget> {
        let busy = self.external.running();
        // A pending launch or a result keeps its tool type until it is finished.
        let session = busy || self.external.pending() || self.external.report.is_some();
        let saving = self.state_pending();
        let state_error = self.state_error.clone();
        let widgets = &mut self.frame_widgets;
        let s = &mut self.external;
        let kinds = [
            (ExternalToolKind::Editor, Icon::Pencil, "Open a working file"),
            (ExternalToolKind::Diff, Icon::Columns, "Compare two versions"),
            (ExternalToolKind::Merge, Icon::Merge, "Resolve a conflict"),
        ];
        let columns = if ui.available_width() < 600.0 { 1 } else { 3 };
        let mut picked = None;
        ui.columns(columns, |cols| {
            for (i, (kind, image, purpose)) in kinds.into_iter().enumerate() {
                let ui = &mut cols[i % columns];
                let count = s.editing.0[kind.index()].entries.len();
                let detail = format!("{count} {}", if count == 1 { "profile" } else { "profiles" });
                let r = kind_card(ui, image, kind.label(), purpose, &detail, s.kind == kind, !session)
                    .on_disabled_hover_text("Finish or clear the current tool session first");
                register(ui, widgets, &format!("tools-kind-{}", kind.index()), &r);
                if r.clicked() {
                    picked = Some(kind);
                }
            }
        });
        if let Some(kind) = picked {
            s.kind = kind;
            s.removing = false;
        }
        ui.add_space(16.0);
        let mut browse = None;
        if ui.available_width() >= 640.0 {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 14.0;
                ui.vertical(|ui| {
                    ui.set_width(236.0);
                    profile_list(ui, widgets, s, busy);
                });
                ui.vertical(|ui| {
                    ui.set_width(ui.available_width());
                    browse = profile_editor(ui, widgets, s, busy);
                });
            });
        } else {
            profile_list(ui, widgets, s, busy);
            ui.add_space(12.0);
            browse = profile_editor(ui, widgets, s, busy);
        }
        ui.add_space(10.0);
        if let Some(error) = &state_error {
            status_line(ui, RED(), error);
        } else if saving {
            ui.label(RichText::new("Saving local settings…").size(12.0).color(HINT()));
        }
        if let Some(error) = &s.error {
            status_line(ui, RED(), error);
        }
        if let Some(notice) = &s.notice {
            ui.add(Label::new(RichText::new(notice).size(12.0).color(MUTED())).wrap());
        }
        browse
    }
}

/// A tool type: icon, name, what it is for and how many profiles it has.
fn kind_card(
    ui: &mut Ui,
    image: Icon,
    title: &str,
    purpose: &str,
    detail: &str,
    selected: bool,
    enabled: bool,
) -> Response {
    let response = ui.add_enabled(
        enabled,
        Button::new("")
            .frame(false)
            .min_size(vec2(ui.available_width(), 60.0)),
    );
    let rect = response.rect;
    let painter = ui.painter();
    let hot = enabled && (response.hovered() || response.has_focus());
    painter.rect(
        rect,
        9.0,
        if selected { ACCENT_SOFT() } else { CARD() },
        Stroke::new(
            1.0,
            if selected {
                BLUE()
            } else if hot {
                EDGE_HOT()
            } else {
                CARD_STROKE()
            },
        ),
        StrokeKind::Inside,
    );
    let tile = Rect::from_min_size(rect.min + vec2(14.0, 14.0), vec2(32.0, 32.0));
    painter.rect_filled(tile, 8.0, if selected { BLUE().gamma_multiply(0.16) } else { HOVER() });
    icon(painter, tile.shrink(8.0), image, if selected { BLUE() } else { MUTED() });
    painter.text(
        pos2(tile.right() + 12.0, rect.top() + 21.0),
        Align2::LEFT_CENTER,
        title,
        semibold(13.0),
        TEXT(),
    );
    let detail_width = painter
        .layout_no_wrap(detail.into(), FontId::proportional(12.0), HINT())
        .size()
        .x;
    let mut job = text::LayoutJob::simple_singleline(purpose.into(), FontId::proportional(12.0), HINT());
    job.wrap = text::TextWrapping::truncate_at_width((rect.right() - tile.right() - 30.0 - detail_width).max(0.0));
    let galley = painter.layout_job(job);
    painter.galley(pos2(tile.right() + 12.0, rect.top() + 32.0), galley, HINT());
    painter.text(
        pos2(rect.right() - 14.0, rect.center().y),
        Align2::RIGHT_CENTER,
        detail,
        FontId::proportional(12.0),
        MUTED(),
    );
    response.widget_info(|| WidgetInfo::selected(WidgetType::Button, enabled, selected, title));
    response
}

fn program_name(executable: &str) -> String {
    let executable = executable.trim();
    if executable.is_empty() {
        return "No program yet".into();
    }
    std::path::Path::new(executable)
        .file_name()
        .map_or_else(|| executable.to_owned(), |n| n.to_string_lossy().into_owned())
}

fn profile_list(ui: &mut Ui, widgets: &mut HashMap<String, UiControl>, s: &mut State, busy: bool) {
    let kind = s.kind;
    Frame::new()
        .fill(CARD())
        .stroke(Stroke::new(1.0, CARD_STROKE()))
        .corner_radius(8)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            Frame::new()
                .inner_margin(Margin {
                    left: 12,
                    right: 12,
                    top: 10,
                    bottom: 6,
                })
                .show(ui, |ui| {
                    ui.label(RichText::new("PROFILES").font(semibold(11.0)).color(HINT()));
                });
            let group = &mut s.editing.0[kind.index()];
            let selected = &mut s.selected[kind.index()];
            let mut picked = None;
            let list = ui.vertical(|ui| {
                for (index, profile) in group.entries.iter().enumerate() {
                    let r = profile_row(
                        ui,
                        &profile.name,
                        &program_name(&profile.config.executable),
                        index == group.default,
                        *selected == index,
                        !busy,
                    );
                    register(ui, widgets, &format!("tools-profile-{index}"), &r);
                    if r.clicked() {
                        picked = Some(index);
                    }
                }
            });
            register(ui, widgets, "tools-profile-picker", &list.response);
            if let Some(index) = picked {
                *selected = index;
                s.removing = false;
            }
            crate::preferences::divider(ui);
            Frame::new().inner_margin(Margin::same(6)).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    let can_add = !busy && group.entries.len() < crate::tool_profiles::MAX_PROFILES;
                    let add = crate::pages::tool(ui, Icon::Plus, "Add", can_add)
                        .on_disabled_hover_text("Keep at most 16 profiles of each type");
                    register(ui, widgets, "tools-profile-add", &add);
                    if !Popup::is_id_open(ui.ctx(), Popup::default_response_id(&add)) {
                        s.known = None;
                    }
                    let layer = ui.layer_id();
                    Popup::menu(&add).show(|ui| {
                        // Modal and popup share a layer order; keep the list above the dialog.
                        ui.ctx().set_sublayer(layer, ui.layer_id());
                        let known = s.known.get_or_insert_with(|| {
                            known_tools::offered(kind).map(|tool| (tool, tool.find())).collect()
                        });
                        if let Some(chosen) = known_menu(ui, widgets, group, known) {
                            let index = match chosen {
                                Some((tool, program)) => group.use_known(*selected, kind, tool, program.as_deref()),
                                None => group.add(kind, None),
                            };
                            if let Some(index) = index {
                                *selected = index;
                            }
                            s.removing = false;
                        }
                    });
                    let duplicate = crate::pages::tool(ui, Icon::Copy, "Duplicate", can_add);
                    register(ui, widgets, "tools-profile-duplicate", &duplicate);
                    if duplicate.clicked() {
                        if let Some(i) = group.add(kind, Some(*selected)) {
                            *selected = i;
                        }
                        s.removing = false;
                    }
                });
            });
        });
}

/// What Add offers for this tool type: what's on this computer, what isn't, and an empty
/// profile. Returns the tool chosen, or None within for Empty profile.
#[allow(clippy::type_complexity)]
fn known_menu(
    ui: &mut Ui,
    widgets: &mut HashMap<String, UiControl>,
    group: &crate::tool_profiles::Group,
    known: &[(&'static KnownTool, Option<PathBuf>)],
) -> Option<Option<(&'static KnownTool, Option<PathBuf>)>> {
    ui.set_width(330.0);
    ui.spacing_mut().item_spacing.y = 0.0;
    let mut chosen = None;
    for (heading, here) in [("ON THIS COMPUTER", true), ("NOT FOUND", false)] {
        let tools: Vec<_> = known.iter().filter(|(_, found)| found.is_some() == here).collect();
        if tools.is_empty() {
            continue;
        }
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            ui.label(RichText::new(heading).font(semibold(11.0)).color(HINT()));
        });
        ui.add_space(2.0);
        for (tool, found) in tools {
            let added = group.holding(tool, found.as_deref()).is_some();
            let r = known_row(ui, tool.name, tool.file, here, added);
            register(ui, widgets, &format!("tools-known-{}", tool.id), &r);
            if r.clicked() {
                chosen = Some(Some((*tool, found.clone())));
            }
        }
        ui.add_space(2.0);
        ui.separator();
    }
    ui.add_space(2.0);
    let r = known_row(ui, "     Empty profile", "", true, false);
    icon(ui.painter(), Rect::from_center_size(pos2(r.rect.left() + 14.0, r.rect.center().y), vec2(12.0, 12.0)), Icon::Plus, HINT());
    register(ui, widgets, "tools-known-empty", &r);
    if r.clicked() {
        chosen = Some(None);
    }
    chosen
}

/// A tool in the Add list: its name, then its program, or that it's in the list already.
fn known_row(ui: &mut Ui, name: &str, file: &str, here: bool, added: bool) -> Response {
    let response = ui.add(Button::new("").frame(false).min_size(vec2(ui.available_width(), CONTROL)));
    let rect = response.rect;
    let painter = ui.painter();
    if response.hovered() || response.has_focus() {
        painter.rect_filled(rect, 6.0, HOVER());
    }
    let left = pos2(rect.left() + 8.0, rect.center().y);
    painter.text(left, Align2::LEFT_CENTER, name, FontId::proportional(13.0), if here { TEXT() } else { HINT() });
    let right = pos2(rect.right() - 8.0, rect.center().y);
    if added {
        let width = painter.text(right, Align2::RIGHT_CENTER, "Added", FontId::proportional(12.0), GREEN()).width();
        let mark = Rect::from_center_size(pos2(right.x - width - 10.0, right.y), vec2(12.0, 12.0));
        icon(painter, mark, Icon::Check, GREEN());
    } else {
        painter.text(right, Align2::RIGHT_CENTER, file, FontId::monospace(11.5), if here { HINT() } else { FAINT() });
    }
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, name));
    response
}

fn profile_row(
    ui: &mut Ui,
    name: &str,
    program: &str,
    default: bool,
    selected: bool,
    enabled: bool,
) -> Response {
    let response = ui.add_enabled(
        enabled,
        Button::new("")
            .frame(false)
            .min_size(vec2(ui.available_width(), 48.0)),
    );
    let rect = response.rect;
    let painter = ui.painter();
    if selected {
        painter.rect_filled(rect, 0.0, SELECTED());
        painter.rect_filled(
            Rect::from_min_size(rect.min + vec2(0.0, 8.0), vec2(2.5, rect.height() - 16.0)),
            1.0,
            BLUE(),
        );
    } else if enabled && (response.hovered() || response.has_focus()) {
        painter.rect_filled(rect, 0.0, HOVER());
    }
    let pill = if default {
        let galley = painter.layout_no_wrap("Default".into(), semibold(11.0), BLUE());
        let pill = Rect::from_min_size(
            pos2(rect.right() - 12.0 - galley.size().x - 14.0, rect.center().y - 9.0),
            vec2(galley.size().x + 14.0, 18.0),
        );
        painter.rect_filled(pill, 9.0, BLUE().gamma_multiply(0.14));
        painter.galley(pos2(pill.left() + 7.0, pill.center().y - galley.size().y / 2.0), galley, BLUE());
        pill.width() + 8.0
    } else {
        0.0
    };
    let width = (rect.width() - 24.0 - pill).max(0.0);
    for (y, value, font, color) in [
        (rect.top() + 16.0, name, semibold(13.0), TEXT()),
        (rect.top() + 33.0, program, FontId::monospace(11.0), HINT()),
    ] {
        let mut job = text::LayoutJob::simple_singleline(value.into(), font, color);
        job.wrap = text::TextWrapping::truncate_at_width(width);
        let galley = painter.layout_job(job);
        painter.galley(pos2(rect.left() + 12.0, y - galley.size().y / 2.0), galley, color);
    }
    response.widget_info(|| WidgetInfo::selected(WidgetType::Button, enabled, selected, name));
    response
}

/// What is wrong with a profile, or that it is ready to launch. `found` is whether
/// its program is there, once that has been checked.
fn profile_check(name: &str, config: &ExternalToolConfig, kind: ExternalToolKind, found: Option<bool>) -> (Color32, String) {
    let executable = config.executable.trim();
    let placeholders = kind.placeholders();
    if executable.is_empty() {
        if let Some(tool) = known_tools::named(name, kind) {
            return (AMBER(), format!("{} wasn't found. Browse to {}.", tool.name, tool.file));
        }
        return (AMBER(), "No program yet, so this profile cannot launch.".into());
    }
    let path = std::path::Path::new(executable);
    if !path.is_absolute()
        || (cfg!(windows) && !path.extension().is_some_and(|x| x.eq_ignore_ascii_case("exe")))
    {
        return (RED(), "Use the full path to the tool's .exe file.".into());
    }
    let lines: Vec<&str> = config.arguments.lines().filter(|l| !l.is_empty()).collect();
    let missing: Vec<&str> = placeholders.iter().copied().filter(|p| !lines.contains(p)).collect();
    if !missing.is_empty() {
        return (
            RED(),
            format!(
                "Add {} on {} own line.",
                missing.join(" and "),
                if missing.len() == 1 { "its" } else { "their" }
            ),
        );
    }
    if let Some(other) = lines
        .iter()
        .find(|a| a.contains(['{', '}']) && !placeholders.contains(a))
    {
        return (
            RED(),
            format!("{other} is not a placeholder here. Use {}.", placeholders.join(", ")),
        );
    }
    match config.validate(kind) {
        Ok(()) if found == Some(false) => (AMBER(), "Program not found. Browse to it again, or remove this profile.".into()),
        Ok(()) => (GREEN(), format!("Ready · passes {}", placeholders.join(", "))),
        Err(_) => (RED(), "Use at most 64 arguments, one per line.".into()),
    }
}

/// One argument per line, with line numbers beside them.
pub(crate) fn arguments_editor(
    ui: &mut Ui,
    widgets: &mut HashMap<String, UiControl>,
    key: &str,
    arguments: &mut String,
    enabled: bool,
) {
    let rows = arguments.lines().count().clamp(3, 12);
    Frame::new()
        .fill(BG())
        .stroke(Stroke::new(1.0, BORDER()))
        .corner_radius(6)
        .inner_margin(Margin {
            left: 0,
            right: 8,
            top: 5,
            bottom: 5,
        })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                let gutter = ui.cursor().left();
                ui.add_space(32.0);
                ui.add_enabled_ui(enabled, |ui| {
                    let output = TextEdit::multiline(arguments)
                        .font(FontId::monospace(12.5))
                        .frame(Frame::NONE)
                        .desired_rows(rows)
                        .desired_width(ui.available_width())
                        .char_limit(16384)
                        .show(ui);
                    register(ui, widgets, key, &output.response.response);
                    let mut line = 1;
                    let mut starts = true;
                    for row in &output.galley.rows {
                        if starts {
                            ui.painter().text(
                                pos2(gutter + 24.0, output.galley_pos.y + row.rect().center().y),
                                Align2::RIGHT_CENTER,
                                line.to_string(),
                                FontId::monospace(11.0),
                                FAINT(),
                            );
                            line += 1;
                        }
                        starts = row.ends_with_newline;
                    }
                });
            });
        });
}

fn profile_editor(
    ui: &mut Ui,
    widgets: &mut HashMap<String, UiControl>,
    s: &mut State,
    busy: bool,
) -> Option<PickTarget> {
    use crate::preferences::{divider, field_row};
    let kind = s.kind;
    let index = s.selected[kind.index()];
    let group = &mut s.editing.0[kind.index()];
    let count = group.entries.len();
    let is_default = index == group.default;
    let mut browse = None;
    let mut make_default = false;
    let wanted = {
        let program = Path::new(group.entries[index].config.executable.trim());
        program.is_absolute().then(|| program.to_path_buf())
    };
    let found = s.program_check.status(ui.ctx(), wanted, None, check_program).map(|status| status == Status::Exists);
    Frame::new()
        .fill(CARD())
        .stroke(Stroke::new(1.0, CARD_STROKE()))
        .corner_radius(8)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            let profile = &mut group.entries[index];
            field_row(ui, "Name", |ui| {
                let r = ui.add_enabled(
                    !busy,
                    TextEdit::singleline(&mut profile.name)
                        .char_limit(80)
                        .desired_width(ui.available_width()),
                );
                register(ui, widgets, "tools-profile-name", &r);
            });
            field_row(ui, "Program", |ui| {
                ui.horizontal(|ui| {
                    let r = ui.add_enabled(
                        !busy,
                        TextEdit::singleline(&mut profile.config.executable)
                            .font(FontId::monospace(12.0))
                            .hint_text("Full path to the tool's .exe")
                            .char_limit(32768)
                            .desired_width((ui.available_width() - 84.0).max(80.0)),
                    );
                    register(ui, widgets, "tools-executable", &r);
                    let b = ui.add_enabled(!busy, Button::new("Browse"));
                    register(ui, widgets, "tools-browse", &b);
                    if b.clicked() {
                        browse = Some(PickTarget::ToolProgram(kind, index));
                    }
                });
            });
            field_row(ui, "Arguments", |ui| {
                arguments_editor(ui, widgets, "tools-arguments", &mut profile.config.arguments, !busy);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    ui.label(RichText::new("Insert").size(12.0).color(HINT()));
                    for (i, placeholder) in kind.placeholders().iter().enumerate() {
                        let r = ui.add_enabled(
                            !busy,
                            quiet(Button::new(RichText::new(*placeholder).monospace().size(12.0).color(BLUE()))),
                        )
                        .on_hover_text(format!("Add {placeholder} as an argument"));
                        register(ui, widgets, &format!("tools-insert-{i}"), &r);
                        if r.clicked() {
                            let arguments = &mut profile.config.arguments;
                            if !arguments.is_empty() && !arguments.ends_with('\n') {
                                arguments.push('\n');
                            }
                            arguments.push_str(placeholder);
                        }
                    }
                });
                let (color, text) = profile_check(&profile.name, &profile.config, kind, found);
                crate::preferences::status_line(ui, color, &text);
            });
            divider(ui);
            Frame::new()
                .inner_margin(Margin::symmetric(16, 10))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        if is_default {
                            let (rect, _) = ui.allocate_exact_size(vec2(13.0, 13.0), Sense::hover());
                            icon(ui.painter(), rect, Icon::Check, BLUE());
                            let what = match kind {
                                ExternalToolKind::Editor => "files",
                                ExternalToolKind::Diff => "comparisons",
                                ExternalToolKind::Merge => "conflicts",
                            };
                            ui.label(RichText::new(format!("Used by default for {what}")).size(12.5).color(MUTED()));
                        } else {
                            make_default = button(ui, widgets, "tools-profile-default", "Make default", !busy);
                        }
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if !s.removing {
                                let r = ui
                                    .add_enabled(
                                        !busy && count > 1,
                                        quiet(Button::new(RichText::new("Remove profile").color(RED()))),
                                    )
                                    .on_disabled_hover_text("Keep at least one profile");
                                register(ui, widgets, "tools-profile-remove", &r);
                                if r.clicked() {
                                    s.removing = true;
                                }
                            }
                        });
                    });
                    if s.removing {
                        ui.add_space(8.0);
                        Frame::new()
                            .fill(RED().gamma_multiply(0.08))
                            .stroke(Stroke::new(1.0, RED().gamma_multiply(0.3)))
                            .corner_radius(7)
                            .inner_margin(Margin::symmetric(12, 7))
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.horizontal(|ui| {
                                    ui.add(
                                        Label::new(
                                            RichText::new("Remove this profile? The program stays installed.")
                                                .size(12.5)
                                                .color(RED()),
                                        )
                                        .truncate(),
                                    );
                                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                        let confirm = crate::pages::danger(ui, !busy, "Remove profile");
                                        register(ui, widgets, "tools-profile-remove-confirm", &confirm);
                                        if confirm.clicked() {
                                            if let Some(i) = group.remove(index) {
                                                s.selected[kind.index()] = i;
                                            }
                                            s.removing = false;
                                        }
                                        if button(ui, widgets, "tools-profile-remove-cancel", "Keep profile", !busy) {
                                            s.removing = false;
                                        }
                                    });
                                });
                            });
                    }
                    ui.add_space(8.0);
                    ui.label(
                        RichText::new("Use a program that stays open until its window closes.")
                            .size(12.0)
                            .color(HINT()),
                    );
                });
        });
    if make_default {
        s.editing.0[kind.index()].default = index;
    }
    browse
}
fn register(ui: &Ui, widgets: &mut HashMap<String, UiControl>, key: &str, r: &Response) {
    let rect = r.interact_rect.intersect(ui.clip_rect());
    if rect.is_positive() {
        widgets.insert(
            key.into(),
            UiControl {
                rect,
                enabled: r.enabled(),
            },
        );
    }
}
fn button(
    ui: &mut Ui,
    widgets: &mut HashMap<String, UiControl>,
    key: &str,
    label: &str,
    enabled: bool,
) -> bool {
    let r = ui.add_enabled(enabled, Button::new(label));
    register(ui, widgets, key, &r);
    r.clicked()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_form_names_what_is_missing() {
        let kind = ExternalToolKind::Merge;
        let empty = |arguments: &str| ExternalToolConfig { executable: String::new(), arguments: arguments.into() };
        let (color, text) = profile_check("WinMerge", &empty("{base}\n{ours}\n{theirs}\n{result}"), kind, None);
        assert_eq!(text, "WinMerge wasn't found. Browse to WinMergeU.exe.");
        assert!(color == AMBER());
        let (_, text) = profile_check("Default", &empty("{base}"), kind, None);
        assert_eq!(text, "No program yet, so this profile cannot launch.");
        let (_, text) = profile_check("Notepad++", &empty("{base}"), kind, None);
        assert_eq!(text, "No program yet, so this profile cannot launch.", "Notepad++ isn't a merge tool");

        let saved = ExternalToolConfig {
            executable: r"C:\Program Files\Beyond Compare 4\BComp.exe".into(),
            arguments: "{ours}\n{theirs}\n{base}\n{result}".into(),
        };
        let (color, text) = profile_check("Beyond Compare", &saved, kind, Some(false));
        assert_eq!(text, "Program not found. Browse to it again, or remove this profile.");
        assert!(color == AMBER());
        for found in [Some(true), None] {
            let (color, text) = profile_check("Beyond Compare", &saved, kind, found);
            assert_eq!(text, "Ready · passes {base}, {ours}, {theirs}, {result}");
            assert!(color == GREEN());
        }
    }
}
