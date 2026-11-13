//! Preview tabs: someone else's repository opened read-only from Rocket's cache.
//! Opening clones it bare without old file versions, then downloads the files of
//! its default branch, so Files, Map and Statistics work at once; other versions
//! download when a view needs them. Keep turns it into a normal clone; Discard,
//! closing the tab or quitting deletes it. Nothing from the repository runs.
use crate::{
    app::*,
    backend::user_error,
    pages::{self, control},
    style::*,
};
use eframe::egui::*;
use rocket_git::*;
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, mpsc},
    time::{Duration, Instant},
};

/// What to preview, from a search result, a pasted address or the GitHub page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreviewRequest {
    pub source: CloneSource,
    /// What origin names once kept: the URL, or the folder.
    pub url: String,
    /// "owner/name", or a folder's name.
    pub name: String,
    pub host: String,
    /// A github.com page, for Open on GitHub.
    pub web: Option<String>,
    /// GitHub's size of the whole repository, when known.
    pub size_kb: Option<u64>,
    /// The default branch, when GitHub said, for "Download the files of main".
    pub branch: Option<String>,
}

pub struct Preview {
    pub request: PreviewRequest,
    pub slot: Arc<PreviewSlot>,
    pub meter: Arc<PreviewDownloads>,
    /// "Continue without a limit" was chosen for this preview.
    pub unlimited: bool,
    pub state: PreviewState,
    /// Sign in on the refused card: Git's credential helper for this host, then a retry.
    pub signing: Option<ReadTask<()>>,
    /// Why the last Sign in didn't finish.
    pub sign_in_error: Option<String>,
}

pub enum PreviewState {
    Opening(Opening),
    Failed(Failure),
    Ready { branch: Option<String> },
}

pub struct Opening {
    task: ReadTask<PreviewOpened>,
    progress: mpsc::Receiver<CloneProgress>,
    pub step: Step,
    pub files: (u64, u64),
    /// Git's "Receiving objects" count for the history, when it gave a total.
    pub history: Option<(u64, u64)>,
    pub started: Instant,
    pub stopping: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Step {
    Connect,
    History,
    Files,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Failure {
    Limit,
    Refused,
    NotFound,
    Offline,
    Stopped,
    Other(String),
}

impl Failure {
    pub fn of(error: &Error) -> Self {
        if matches!(error, Error::Cancelled) {
            return Self::Stopped;
        }
        match error.clone_failure() {
            Some(CloneFailure::SizeLimit) => Self::Limit,
            Some(CloneFailure::Transport(
                FetchFailure::AuthenticationRejected
                | FetchFailure::CredentialsUnavailable
                | FetchFailure::AccessDenied
                | FetchFailure::SsoRequired
                | FetchFailure::SshAuthenticationRejected
                | FetchFailure::SshKeyUnavailable,
            )) => Self::Refused,
            Some(CloneFailure::Transport(FetchFailure::RepositoryUnavailable))
            | Some(CloneFailure::SourceUnavailable) => Self::NotFound,
            Some(CloneFailure::Transport(
                FetchFailure::NameResolutionFailed
                | FetchFailure::ConnectionFailed
                | FetchFailure::ConnectionTimedOut
                | FetchFailure::ConnectionInterrupted,
            )) => Self::Offline,
            _ => Self::Other(user_error(error)),
        }
    }
    pub fn code(&self) -> &'static str {
        match self {
            Self::Limit => "limit",
            Self::Refused => "refused",
            Self::NotFound => "not_found",
            Self::Offline => "offline",
            Self::Stopped => "stopped",
            Self::Other(_) => "other",
        }
    }
}

impl Preview {
    pub fn ready(&self) -> bool {
        matches!(self.state, PreviewState::Ready { .. })
    }
    pub fn opening(&self) -> bool {
        matches!(self.state, PreviewState::Opening(_))
    }
    /// For automation and the banner: "opening", "ready" or the failure's code.
    pub fn state_name(&self) -> &'static str {
        match &self.state {
            PreviewState::Opening(_) => "opening",
            PreviewState::Ready { .. } => "ready",
            PreviewState::Failed(failure) => failure.code(),
        }
    }
    /// "fuzzyfind" from "acme/fuzzyfind".
    pub fn short_name(&self) -> &str {
        self.request.name.rsplit('/').next().unwrap_or(&self.request.name)
    }
}

/// Repository size on GitHub, from its kilobytes.
fn kilobytes(kb: u64) -> u64 {
    kb.saturating_mul(1024)
}

#[derive(Default)]
struct Card {
    stop: bool,
    sign_in: bool,
    retry: Option<bool>,
    clone: bool,
    close: bool,
    edit: bool,
    open: bool,
}

impl Rocket {
    /// Rocket's preview folder: inside the test folder for an MCP window, else next
    /// to Rocket's state folder.
    pub(crate) fn preview_cache(&mut self) -> Result<PreviewCache> {
        if let Some(cache) = &self.preview_cache {
            return Ok(cache.clone());
        }
        let cache = PreviewCache::open(self.preview_root())?;
        // Previews a crashed Rocket left here. The startup pass looks in the usual
        // folder, which a test window replaces after it starts.
        let stale = cache.clone();
        let _ = std::thread::Builder::new()
            .name("rocket-preview-cleanup".into())
            .spawn(move || stale.remove_stale());
        self.preview_cache = Some(cache.clone());
        Ok(cache)
    }

    pub(crate) fn preview_root(&self) -> PathBuf {
        match &self.browse_root {
            Some(root) => root.join("rocket-previews"),
            None => self
                .state_directory
                .parent()
                .map_or_else(|| self.state_directory.join("previews"), |p| p.join("previews")),
        }
    }

    /// Open a preview in a new tab, or show the tab already previewing it.
    pub(crate) fn start_preview(&mut self, request: PreviewRequest, unlimited: bool) {
        if self.quitting {
            return;
        }
        if let Some(index) = self
            .tabs
            .iter()
            .position(|t| t.preview.as_ref().is_some_and(|p| p.request.url == request.url))
        {
            self.active = index;
            self.close_explore_page();
            return;
        }
        let slot = match self.preview_cache().and_then(|cache| cache.reserve()) {
            Ok(slot) => slot,
            Err(error) => {
                self.explore.problem = Some(crate::explore::Problem::Failed(user_error(&error)));
                return;
            }
        };
        let limit = (!unlimited).then_some(self.preview_limit);
        let mut tab = Tab::new(slot.repository(), self.backend.as_ref().map(|b| b.log.clone()));
        tab.title = request.name.rsplit('/').next().unwrap_or(&request.name).to_owned();
        let failed = PreviewState::Failed(Failure::Stopped);
        tab.preview = Some(Box::new(Preview {
            request,
            slot: Arc::new(slot),
            meter: PreviewDownloads::new(limit),
            unlimited,
            state: failed,
            signing: None,
            sign_in_error: None,
        }));
        let id = tab.id;
        self.tabs.push(tab);
        self.active = self.tabs.len() - 1;
        self.close_explore_page();
        self.spawn_preview(id);
    }

    fn close_explore_page(&mut self) {
        if self.dialog.as_ref().is_some_and(|d| matches!(d.kind, DialogKind::Explore)) {
            self.dialog = None;
        }
    }

    /// (Re)start downloading the preview of tab `id`.
    fn spawn_preview(&mut self, id: u64) {
        let Some(backend) = self.backend.clone() else {
            return;
        };
        let preview_limit = self.preview_limit;
        let context = self.context.clone();
        let Some(preview) = self.tabs.iter_mut().find(|t| t.id == id).and_then(|t| t.preview.as_mut()) else {
            return;
        };
        let limit = (!preview.unlimited).then_some(preview_limit);
        preview.meter = PreviewDownloads::new(limit);
        let (tx, rx) = mpsc::sync_channel(64);
        let source = preview.request.source.clone();
        let slot = preview.slot.clone();
        let meter = preview.meter.clone();
        match crate::read_task::spawn(&context, move |cancel| {
            slot.clear()?;
            backend.open_preview(&source, &slot, limit, &meter, &tx, &cancel)
        }) {
            Ok(task) => {
                preview.state = PreviewState::Opening(Opening {
                    task,
                    progress: rx,
                    step: Step::Connect,
                    files: (0, 0),
                    history: None,
                    started: Instant::now(),
                    stopping: false,
                })
            }
            Err(error) => preview.state = PreviewState::Failed(Failure::Other(user_error(&error))),
        }
    }

    pub(crate) fn poll_previews(&mut self, ctx: &Context) {
        let mut opened = Vec::new();
        let mut waiting = false;
        let mut signed = Vec::new();
        for tab in &mut self.tabs {
            let Some(preview) = tab.preview.as_mut() else {
                continue;
            };
            let Some(result) = preview.signing.as_mut().and_then(|task| task.try_finish()) else {
                waiting |= preview.signing.is_some();
                continue;
            };
            preview.signing = None;
            match result {
                Ok(()) => signed.push(tab.id),
                Err(Error::Cancelled) => {}
                Err(error) => preview.sign_in_error = Some(user_error(&error)),
            }
        }
        for id in signed {
            self.spawn_preview(id);
        }
        for tab in &mut self.tabs {
            let Some(preview) = tab.preview.as_mut() else {
                continue;
            };
            let PreviewState::Opening(opening) = &mut preview.state else {
                continue;
            };
            waiting = true;
            while let Ok(event) = opening.progress.try_recv() {
                match event {
                    CloneProgress::Transfer(progress) => {
                        opening.step = opening.step.max(Step::History);
                        if progress.phase == FetchPhase::Receiving
                            && let Some(total) = progress.total.filter(|t| *t > 0)
                        {
                            opening.history = Some((progress.current.min(total), total));
                        }
                    }
                    CloneProgress::Phase(ClonePhase::DownloadingFiles) => opening.step = Step::Files,
                    CloneProgress::Files { done, total } => {
                        opening.step = Step::Files;
                        opening.files = (done, total);
                    }
                    _ => {}
                }
            }
            let Some(result) = opening.task.try_finish() else {
                continue;
            };
            match result {
                Ok(done) => {
                    preview.meter.set_size(done.size);
                    preview.state = PreviewState::Ready { branch: done.branch };
                    opened.push((tab.id, done.repository, preview.meter.clone()));
                }
                Err(error) => preview.state = PreviewState::Failed(Failure::of(&error)),
            }
        }
        if waiting {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
        for (id, repository, meter) in opened {
            if let Some(backend) = &self.backend
                && let Err(error) = backend.register_preview(&repository, meter)
            {
                if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == id)
                    && let Some(preview) = tab.preview.as_mut()
                {
                    preview.state = PreviewState::Failed(Failure::Other(user_error(&error)));
                }
                continue;
            }
            self.refresh(id);
        }
        // Keep the banner's download count moving while a view downloads.
        if self.tab().and_then(|t| t.preview.as_ref()).is_some_and(|p| p.meter.status().running) {
            ctx.request_repaint_after(Duration::from_millis(300));
        }
    }

    /// Sign in on the refused card: ask Git's credential helper to sign in to the
    /// preview's host, with its own window when it has one, then try again.
    fn preview_sign_in(&mut self, id: u64) {
        let Some(backend) = self.backend.clone() else {
            return;
        };
        let context = self.context.clone();
        let Some(preview) = self.tabs.iter_mut().find(|t| t.id == id).and_then(|t| t.preview.as_mut()) else {
            return;
        };
        if preview.signing.is_some() {
            return;
        }
        let host = preview.request.host.clone();
        preview.sign_in_error = None;
        match crate::read_task::spawn(&context, move |cancel| backend.preview_sign_in(&host, &cancel)) {
            Ok(task) => preview.signing = Some(task),
            Err(error) => preview.sign_in_error = Some(user_error(&error)),
        }
    }

    /// Activity's line for a preview the user discarded or closed.
    pub(crate) fn note_discard(&mut self, preview: &Preview) {
        let freed = preview.meter.status().size;
        let outcome = crate::backend::Outcome {
            command_succeeded: true,
            summary: format!("Discarded the preview of {} · {} freed", preview.request.name, crate::explore::size(freed.max(1))),
            ..Default::default()
        };
        self.activities.push(Activity {
            title: "discard-preview".into(),
            outcome,
            elapsed: Duration::ZERO,
            at: std::time::SystemTime::now(),
            seen: false,
        });
        self.activity_history.mark_dirty();
        if self.activities.len() > 40 {
            self.activities.remove(0);
        }
    }

    /// Stop a preview that is still downloading.
    pub(crate) fn stop_preview(&mut self, id: u64) {
        if let Some(preview) = self.tabs.iter_mut().find(|t| t.id == id).and_then(|t| t.preview.as_mut())
            && let PreviewState::Opening(opening) = &mut preview.state
        {
            opening.task.cancel();
            opening.stopping = true;
        }
    }

    /// Forget a closed preview: no more downloads, and its folder deleted on a worker.
    /// Files a stopping Git process still holds are retried, then left for the next
    /// start, which removes folders nobody holds.
    pub(crate) fn release_preview(&mut self, preview: Box<Preview>) {
        let repository = preview.slot.repository();
        if let Some(backend) = &self.backend {
            backend.unregister_preview(&repository);
        }
        let slot = preview.slot.clone();
        drop(preview);
        let _ = std::thread::Builder::new()
            .name("rocket-preview-discard".into())
            .spawn(move || {
                for _ in 0..20 {
                    if slot.discard().is_ok() {
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(300));
                }
            });
    }

    /// Delete every open preview while Rocket closes, waiting a short while for each.
    pub(crate) fn release_all_previews(&mut self) {
        for tab in &mut self.tabs {
            if let Some(preview) = tab.preview.take() {
                if let Some(backend) = &self.backend {
                    backend.unregister_preview(&preview.slot.repository());
                }
                let slot = preview.slot.clone();
                drop(preview);
                for _ in 0..5 {
                    if slot.discard().is_ok() {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
        }
    }

    /// The opening or failed card that stands in for a preview's views. Returns
    /// whether it was drawn.
    pub(crate) fn preview_body(&mut self, ui: &mut Ui) -> bool {
        let Some(tab) = self.tabs.get(self.active) else {
            return false;
        };
        let Some(preview) = tab.preview.as_deref() else {
            return false;
        };
        if preview.ready() {
            return false;
        }
        let id = tab.id;
        let can_clone = self.write.is_none();
        let limit_text = if preview.unlimited {
            "No size limit".to_owned()
        } else {
            format!("Stops at {}", crate::explore::size(self.preview_limit))
        };
        let widgets = &mut self.frame_widgets;
        let mut card = Card::default();
        let top = ((ui.available_height() - 360.0) / 2.0).max(16.0);
        ui.add_space(top);
        ui.vertical_centered(|ui| {
            Frame::new()
                .fill(PANEL())
                .stroke(Stroke::new(1.0, HAIRLINE()))
                .corner_radius(10)
                .inner_margin(Margin::symmetric(24, 20))
                .show(ui, |ui| {
                    ui.set_width(520.0f32.min(ui.available_width() - 48.0).max(260.0));
                    ui.with_layout(Layout::top_down(Align::Min), |ui| match &preview.state {
                        PreviewState::Opening(opening) => opening_card(ui, preview, opening, &limit_text, widgets, &mut card),
                        PreviewState::Failed(failure) => failed_card(ui, preview, failure, can_clone, widgets, &mut card),
                        PreviewState::Ready { .. } => {}
                    });
                });
        });
        let (url, web) = (preview.request.url.clone(), preview.request.web.clone());
        if card.stop {
            self.stop_preview(id);
        }
        if card.sign_in {
            self.preview_sign_in(id);
        }
        if let Some(unlimited) = card.retry {
            if let Some(preview) = self.tabs.iter_mut().find(|t| t.id == id).and_then(|t| t.preview.as_mut()) {
                preview.unlimited |= unlimited;
            }
            self.spawn_preview(id);
        }
        if card.clone {
            self.show_dialog(DialogKind::Clone);
            if let Some(d) = &mut self.dialog {
                d.first = url.clone();
            }
        }
        if card.open
            && let Some(web) = web
        {
            self.open_github(&web);
        }
        if card.edit {
            self.dispatch(Intent::CloseTab(id));
            self.open_explore(Some(url));
        } else if card.close {
            self.dispatch(Intent::CloseTab(id));
        }
        true
    }

    /// A preview's toolbar: the branch shown, Refresh, the command search and
    /// Activity. No Fetch, Pull, Push, Stash, Branch or Terminal.
    pub(crate) fn preview_toolbar(&mut self, ui: &mut Ui, intents: &mut Vec<Intent>, compact: bool) {
        let Some(tab) = self.tabs.get(self.active) else {
            return;
        };
        // The branch Commits shows: one picked here or in the sidebar, else the default.
        let branch = match (&tab.history_ref, &tab.snapshot) {
            (Some(reference), _) => reference.strip_prefix("refs/heads/").unwrap_or(reference).to_owned(),
            (None, Some(_)) => tab.branch_label(),
            (None, None) => "…".into(),
        };
        let branches: Vec<(String, String)> = tab
            .snapshot
            .as_ref()
            .map(|s| {
                s.branches
                    .iter()
                    .filter(|b| b.kind == BranchKind::Local)
                    .filter_map(|b| String::from_utf8(b.reference.clone()).ok())
                    .map(|reference| (reference.strip_prefix("refs/heads/").unwrap_or(&reference).to_owned(), reference))
                    .collect()
            })
            .unwrap_or_default();
        let ready = tab.preview.as_ref().is_some_and(|p| p.ready());
        let loading = tab.loading();
        ui.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing = vec2(4.0, 0.0);
            let width = if compact { 150.0 } else { 210.0 };
            let (rect, response) = ui.allocate_exact_size(vec2(width, 38.0), if ready { Sense::click() } else { Sense::hover() });
            let response = response.on_hover_text("The branch whose history Commits shows");
            self.frame_widgets.insert("preview-branch".into(), (&response).into());
            if response.hovered() && ready {
                ui.painter().rect_filled(rect, 7.0, HOVER());
            }
            icon(ui.painter(), Rect::from_center_size(pos2(rect.right() - 12.0, rect.center().y), vec2(10.0, 10.0)), Icon::Chevron, FAINT());
            Popup::menu(&response).show(|ui| {
                ui.set_min_width(200.0);
                ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                    for (name, reference) in &branches {
                        let r = ui.add(Button::selectable(*name == branch, name.as_str()));
                        self.frame_widgets.insert(format!("preview-branch-{name}"), (&r).into());
                        if r.clicked() {
                            intents.push(Intent::History(Some(reference.clone())));
                            ui.close();
                        }
                    }
                });
            });
            let painter = ui.painter();
            let badge = Rect::from_center_size(pos2(rect.left() + 19.0, rect.center().y), vec2(28.0, 28.0));
            painter.rect_filled(badge, 7.0, VIOLET().gamma_multiply(0.16));
            icon(painter, badge.shrink(7.0), Icon::Eye, VIOLET());
            let left = badge.right() + 10.0;
            painter.text(pos2(left, rect.center().y - 8.0), Align2::LEFT_CENTER, "Viewing branch", FontId::proportional(11.0), FAINT());
            let mut job = text::LayoutJob::simple_singleline(branch, semibold(13.0), TEXT());
            job.wrap = text::TextWrapping::truncate_at_width(rect.right() - left - 22.0);
            painter.galley(pos2(left, rect.center().y + 1.0), painter.layout_job(job), TEXT());
            let enabled = ready && !loading;
            let refresh = crate::chrome::icon_button(ui, Icon::Refresh, enabled, false, 32.0)
                .on_hover_text("Refresh (F5)");
            self.frame_widgets.insert("refresh".into(), (&refresh).into());
            if refresh.clicked() {
                intents.push(Intent::Refresh);
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                let open = !self.drawer_collapsed;
                let activity = crate::chrome::icon_button(ui, Icon::Activity, true, self.show_console && open, 32.0)
                    .on_hover_text("Activity (Ctrl+`)");
                self.frame_widgets.insert("toolbar-console".into(), (&activity).into());
                if activity.clicked() {
                    self.toggle_activity();
                }
                ui.add_space(4.0);
                let search = crate::chrome::command_field(ui, compact);
                self.frame_widgets.insert("toolbar-palette".into(), (&search).into());
                if search.clicked() {
                    self.palette.show();
                }
            });
        });
    }

    /// The strip under a ready preview's toolbar: what it is, its size, and Keep.
    pub(crate) fn preview_banner(&mut self, ui: &mut Ui) {
        let Some(tab) = self.tabs.get(self.active) else {
            return;
        };
        let Some(preview) = tab.preview.as_deref() else {
            return;
        };
        let id = tab.id;
        let status = preview.meter.status();
        let keep_ready = self.write.is_none() && preview.ready() && !tab.loading();
        let text = format!(
            "Read-only copy of {}{} · {}",
            if preview.request.host == "this computer" { String::new() } else { format!("{}/", preview.request.host) },
            preview.request.name,
            crate::explore::size(status.size.max(1)),
        );
        let (mut back, mut discard, mut keep) = (false, false, false);
        let widgets = &mut self.frame_widgets;
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 40.0), Sense::hover());
        ui.painter().rect_filled(rect, 0.0, VIOLET().gamma_multiply(0.07));
        ui.painter().hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0, VIOLET().gamma_multiply(0.28)));
        let mut child = ui.new_child(UiBuilder::new().max_rect(rect.shrink2(vec2(14.0, 0.0))).layout(Layout::left_to_right(Align::Center)));
        child.spacing_mut().item_spacing.x = 8.0;
        let (eye, _) = child.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
        icon(child.painter(), eye, Icon::Eye, VIOLET());
        child.label(RichText::new("Preview").font(semibold(13.0)).color(TEXT()));
        child.add(Label::new(RichText::new(text).color(MUTED())).truncate());
        if status.running {
            child.spinner();
            child.label(RichText::new(format!("Downloading {} files", status.files.max(1))).size(12.0).color(MUTED()));
        }
        child.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            let r = primary_icon(ui, Icon::Bookmark, "Keep", keep_ready)
                .on_hover_text("Make a normal clone in a folder you choose");
            control(widgets, "preview-keep", &r, ui.clip_rect());
            keep = r.clicked();
            let r = pages::tool(ui, Icon::Trash, "Discard", true).on_hover_text("Close this tab and delete the preview");
            control(widgets, "preview-discard", &r, ui.clip_rect());
            discard = r.clicked();
            let r = pages::tool(ui, Icon::Search, "Back to results", true);
            control(widgets, "preview-back", &r, ui.clip_rect());
            back = r.clicked();
        });
        if keep {
            self.show_dialog(DialogKind::KeepPreview(id));
        } else if discard {
            self.dispatch(Intent::CloseTab(id));
        } else if back {
            self.open_explore(None);
        }
    }

    /// Start Keep for preview tab `id`: the write that turns it into a clone.
    pub(crate) fn keep_preview(&mut self, id: u64, destination: PathBuf) {
        if self.write.is_some() || self.quitting {
            return;
        }
        let Some(backend) = self.backend.clone() else {
            return;
        };
        let Some(preview) = self.tabs.iter().find(|t| t.id == id).and_then(|t| t.preview.as_ref()) else {
            return;
        };
        let (repository, url, name) = (preview.slot.repository(), preview.request.url.clone(), preview.request.name.clone());
        let (tx, rx) = mpsc::sync_channel(64);
        let (_, fetch_rx) = mpsc::sync_channel(1);
        let (_, force_push_rx) = mpsc::sync_channel(1);
        match WriteTask::spawn(move |cancel| backend.keep_preview(repository, url, name, destination, &tx, &cancel)) {
            Ok(task) => {
                self.keeping_preview = Some(id);
                self.write = Some(ActiveWrite {
                    owner: None,
                    label: "keep-preview".into(),
                    title: crate::operation_names::running("keep-preview"),
                    task,
                    fetch_progress: fetch_rx,
                    clone_progress: rx,
                    force_push_progress: force_push_rx,
                    force_push_phase: None,
                    progress: "Preparing…".into(),
                    fraction: None,
                    cancelling: false,
                    started: Instant::now(),
                    submitted_draft: None,
                });
            }
            Err(error) => self.notice = Some(user_error(&error)),
        }
    }

    /// After Keep: the kept clone replaces the preview's tab.
    pub(crate) fn finish_keep(&mut self, succeeded: bool, summary: &str) {
        let Some(id) = self.keeping_preview.take() else {
            return;
        };
        let keeping = self.dialog.as_ref().is_some_and(|d| matches!(d.kind, DialogKind::KeepPreview(k) if k == id));
        if succeeded {
            if keeping {
                self.dialog = None;
            }
            if let Some(preview) = self.tabs.iter_mut().find(|t| t.id == id).and_then(|t| t.preview.take()) {
                self.release_preview(preview);
            }
            self.remove_tab(id);
        } else if keeping && let Some(d) = &mut self.dialog {
            d.error = Some(summary.to_owned());
        }
    }
}

fn step_row(ui: &mut Ui, state: u8, title: &str, detail: Option<String>, fraction: Option<f32>) {
    // state: 0 waiting, 1 running, 2 done
    ui.horizontal(|ui| {
        ui.set_min_height(36.0);
        let (mark, _) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::hover());
        let painter = ui.painter();
        match state {
            2 => {
                painter.circle_filled(mark.center(), 11.0, GREEN().gamma_multiply(0.18));
                icon(painter, mark.shrink(6.0), Icon::Check, GREEN());
            }
            1 => {
                painter.circle_stroke(mark.center(), 9.5, Stroke::new(2.0, BLUE()));
            }
            _ => {
                painter.circle_stroke(mark.center(), 9.5, Stroke::new(1.5, HAIRLINE()));
            }
        }
        ui.add_space(6.0);
        let color = if state == 0 { HINT() } else { TEXT() };
        let text = RichText::new(title).color(color);
        ui.label(if state == 1 { text.strong() } else { text });
        if let Some(detail) = detail {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(RichText::new(detail).size(12.0).color(HINT()));
            });
        }
    });
    if let Some(fraction) = fraction {
        ui.horizontal(|ui| {
            ui.add_space(34.0);
            ui.add(ProgressBar::new(fraction.clamp(0.0, 1.0)).desired_height(6.0).fill(BLUE()));
        });
        ui.add_space(4.0);
    }
}

fn opening_card(
    ui: &mut Ui,
    preview: &Preview,
    opening: &Opening,
    limit_text: &str,
    widgets: &mut HashMap<String, UiControl>,
    card: &mut Card,
) {
    title(ui, Icon::Eye, VIOLET(), &format!("Previewing {}", preview.request.name), &format!("{} · read-only copy", preview.request.host));
    ui.add_space(10.0);
    let size = preview.meter.status().size;
    let seconds = opening.started.elapsed().as_secs_f32().max(0.5);
    let rate = format!("{} · {}/s", crate::explore::size(size.max(1)), crate::explore::size((size as f32 / seconds) as u64 + 1));
    let state = |step: Step| -> u8 {
        if opening.step > step { 2 } else if opening.step == step { 1 } else { 0 }
    };
    step_row(ui, state(Step::Connect).max(u8::from(opening.step > Step::Connect) * 2), &format!("Connect to {}", preview.request.host), None, None);
    let history = opening.history.map(|(done, total)| done as f32 / total as f32);
    step_row(ui, state(Step::History), "Download the history", (opening.step == Step::History).then(|| rate.clone()), history.filter(|_| opening.step == Step::History));
    let branch = preview.request.branch.as_deref().unwrap_or("the default branch");
    let (done, total) = opening.files;
    let files = (opening.step == Step::Files && total > 0).then(|| format!("{} of {} files · {}", crate::explore::group(done), crate::explore::group(total), crate::explore::size(size.max(1))));
    let fraction = (opening.step == Step::Files && total > 0).then(|| done as f32 / total as f32);
    step_row(ui, state(Step::Files), &format!("Download the files of {branch}"), files, fraction);
    step_row(ui, 0, "Open", None, None);
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.label(RichText::new(limit_text).size(12.0).color(HINT()));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let stop = ui.add_enabled(!opening.stopping, Button::new(if opening.stopping { "Stopping" } else { "Stop" }));
            control(widgets, "preview-stop", &stop, ui.clip_rect());
            card.stop = stop.clicked();
        });
    });
}

fn failed_card(
    ui: &mut Ui,
    preview: &Preview,
    failure: &Failure,
    can_clone: bool,
    widgets: &mut HashMap<String, UiControl>,
    card: &mut Card,
) {
    let host = &preview.request.host;
    let name = &preview.request.name;
    let (image, color, head, text) = match failure {
        Failure::Limit => (
            Icon::Warning,
            AMBER(),
            preview.meter.limit().map_or_else(|| "Stopped at the preview limit".to_owned(), |limit| format!("Stopped at {}", crate::explore::size(limit))),
            match preview.request.size_kb {
                Some(kb) if kb > 0 => format!("{name} is about {} on {host}; its history passed the limit. Nothing was kept.", crate::explore::size(kilobytes(kb))),
                _ => format!("{name} has more history than the preview limit. Nothing was kept."),
            },
        ),
        Failure::Refused => (Icon::Lock, AMBER(), format!("{host} refused access"), format!("{name} is private, or Git's sign-in for {host} can't read it.")),
        Failure::NotFound => (Icon::Warning, RED(), "Repository not found".to_owned(), format!("{} doesn't exist, or you can't see it.", preview.request.url)),
        Failure::Offline => (Icon::Remote, FAINT(), format!("Can't reach {host}"), "Check your connection, then try again.".to_owned()),
        Failure::Stopped => (Icon::Stop, FAINT(), "Preview stopped".to_owned(), "Nothing was kept.".to_owned()),
        Failure::Other(detail) => (Icon::Warning, RED(), "Preview failed".to_owned(), detail.clone()),
    };
    title(ui, image, color, &head, "");
    ui.add_space(4.0);
    ui.label(RichText::new(text).color(TEXT()));
    if let (Failure::Refused, Some(error)) = (failure, &preview.sign_in_error) {
        ui.add_space(4.0);
        ui.label(RichText::new(format!("Sign-in didn't finish: {error}")).size(12.5).color(RED()));
    }
    ui.add_space(12.0);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let mut button = |ui: &mut Ui, key: &str, label: &str, primary: bool, enabled: bool| {
            let r = if primary { pages::primary(ui, enabled, label) } else { pages::ghost(ui, enabled, label) };
            control(widgets, key, &r, ui.clip_rect());
            r.clicked()
        };
        match failure {
            Failure::Limit => {
                card.clone = button(ui, "preview-clone-instead", "Clone instead", true, can_clone);
                if button(ui, "preview-unlimited", "Continue without a limit", false, true) {
                    card.retry = Some(true);
                }
            }
            Failure::Refused => {
                // Git's own sign-in for an https host; SSH addresses use keys instead.
                if preview.request.url.starts_with("https://") {
                    let signing = preview.signing.is_some();
                    card.sign_in = button(ui, "preview-sign-in", if signing { "Signing in" } else { "Sign in" }, true, !signing);
                } else if button(ui, "preview-retry", "Try again", true, true) {
                    card.retry = Some(false);
                }
                if preview.request.web.is_some() {
                    let r = pages::tool(ui, Icon::External, "Open on GitHub", true);
                    control(widgets, "preview-open-github", &r, ui.clip_rect());
                    card.open = r.clicked();
                }
            }
            Failure::NotFound => {
                card.edit = button(ui, "preview-edit", "Edit address", true, true);
                card.close = button(ui, "preview-close", "Close tab", false, true);
            }
            Failure::Offline | Failure::Stopped | Failure::Other(_) => {
                if button(ui, "preview-retry", "Try again", true, true) {
                    card.retry = Some(false);
                }
                card.close = button(ui, "preview-close", "Close tab", false, true);
            }
        }
    });
}

/// The primary button with an icon before its label, as Keep on the banner.
fn primary_icon(ui: &mut Ui, image: Icon, label: &str, enabled: bool) -> Response {
    let font = semibold(13.0);
    let width = ui.painter().layout_no_wrap(label.into(), font.clone(), ACCENT_INK()).size().x;
    let response = ui.add_enabled(enabled, Button::new("").fill(BLUE()).min_size(vec2(width + 40.0, 28.0)));
    let rect = response.rect;
    let ink = if enabled { ACCENT_INK() } else { ACCENT_INK().gamma_multiply(0.55) };
    icon(ui.painter(), Rect::from_center_size(pos2(rect.left() + 16.0, rect.center().y), vec2(13.0, 13.0)), image, ink);
    ui.painter().text(pos2(rect.left() + 28.0, rect.center().y), Align2::LEFT_CENTER, label, font, ink);
    response.widget_info(|| WidgetInfo::labeled(Role::Button, enabled, label));
    response
}

/// What a view needs to show its preview's downloads, read before the view borrows
/// its tab.
#[derive(Clone, Debug)]
pub(crate) struct Downloads {
    pub status: DownloadStatus,
    pub host: String,
    /// The default branch, whose files are already here.
    pub branch: Option<String>,
}

/// The active tab's preview downloads, when the tab is a preview.
pub(crate) fn downloads(tab: &Tab) -> Option<Downloads> {
    let preview = tab.preview.as_deref()?;
    let branch = match &preview.state {
        PreviewState::Ready { branch } => branch.clone(),
        _ => preview.request.branch.clone(),
    };
    Some(Downloads { status: preview.meter.status(), host: preview.request.host.clone(), branch })
}

/// While a preview downloads what a view needs: the card in place of the view's own
/// loading state, with what has arrived and Stop. Returns whether Stop was clicked,
/// or None when nothing is downloading.
pub(crate) fn downloading(
    ui: &mut Ui,
    downloads: Option<&Downloads>,
    title: &str,
    stop: &str,
    widgets: &mut HashMap<String, UiControl>,
) -> Option<bool> {
    let d = downloads.filter(|d| d.status.running)?;
    let s = d.status;
    let arrived = s.received.min(s.files);
    let detail = match (s.files, s.downloaded) {
        (0, _) => format!("Starting the download from {}", d.host),
        (files, 0) => format!("{} of {} from {}", crate::explore::group(arrived), crate::explore::group(files), d.host),
        (files, bytes) => format!("{} of {} · {} so far from {}", crate::explore::group(arrived), crate::explore::group(files), crate::explore::size(bytes), d.host),
    };
    let mut clicked = false;
    pages::loading_with(ui, title, 72.0, |ui| {
        ui.add_space(2.0);
        ui.label(RichText::new(&detail).size(12.5).color(MUTED()));
        if s.files > 0 {
            ui.add_space(4.0);
            ui.add(ProgressBar::new(arrived as f32 / s.files as f32).desired_width(260.0).desired_height(6.0).fill(BLUE()));
        }
        ui.add_space(6.0);
        let r = ui.button("Stop");
        pages::control(widgets, stop, &r, ui.clip_rect());
        clicked = r.clicked();
    });
    ui.ctx().request_repaint_after(Duration::from_millis(250));
    Some(clicked)
}

/// After a preview couldn't download what a view needs: what still works, and Try
/// again. Returns whether Try again was clicked, or None when nothing failed.
pub(crate) fn not_downloaded(
    ui: &mut Ui,
    downloads: Option<&Downloads>,
    retry: &str,
    widgets: &mut HashMap<String, UiControl>,
) -> Option<bool> {
    let d = downloads.filter(|d| !d.status.running)?;
    let failure = d.status.failure?;
    let why = match failure {
        FetchFailure::NameResolutionFailed
        | FetchFailure::ConnectionFailed
        | FetchFailure::ConnectionTimedOut
        | FetchFailure::ConnectionInterrupted => format!("{} can't be reached.", d.host),
        FetchFailure::AuthenticationRejected
        | FetchFailure::CredentialsUnavailable
        | FetchFailure::AccessDenied
        | FetchFailure::SsoRequired
        | FetchFailure::SshAuthenticationRejected
        | FetchFailure::SshKeyUnavailable => format!("{} refused the download.", d.host),
        other => other.summary().to_owned(),
    };
    let still = match &d.branch {
        Some(branch) => format!("History and the files of {branch} still work."),
        None => "History still works.".to_owned(),
    };
    let width = ui.available_width();
    let height = ui.available_height();
    let height = if height.is_finite() && height < 4000.0 { height.max(160.0) } else { 180.0 };
    let mut clicked = false;
    ui.allocate_ui_with_layout(vec2(width, height), Layout::top_down(Align::Center), |ui| {
        ui.set_min_size(vec2(width, height));
        ui.add_space(((height - 150.0) / 2.0).max(8.0));
        let (tile, _) = ui.allocate_exact_size(vec2(36.0, 36.0), Sense::hover());
        ui.painter().rect_filled(tile, 9.0, RAISED());
        icon(ui.painter(), tile.shrink(10.0), Icon::Remote, FAINT());
        ui.add_space(8.0);
        ui.label(RichText::new("Not downloaded yet").font(semibold(14.0)).color(TEXT()));
        ui.add_space(2.0);
        ui.label(RichText::new(format!("{why} {still}")).size(12.5).color(MUTED()));
        ui.add_space(8.0);
        let r = ui.button("Try again");
        pages::control(widgets, retry, &r, ui.clip_rect());
        clicked = r.clicked();
    });
    Some(clicked)
}

fn title(ui: &mut Ui, image: Icon, color: Color32, head: &str, sub: &str) {
    ui.horizontal(|ui| {
        let (tile, _) = ui.allocate_exact_size(vec2(40.0, 40.0), Sense::hover());
        ui.painter().rect_filled(tile, 10.0, color.gamma_multiply(0.14));
        icon(ui.painter(), tile.shrink(11.0), image, color);
        ui.add_space(6.0);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.label(RichText::new(head).font(semibold(16.0)));
            if !sub.is_empty() {
                ui.label(RichText::new(sub).size(12.5).color(MUTED()));
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failures_name_what_the_user_can_do() {
        let git = |stderr: &str| Error::Git { operation: "clone", code: Some(128), stdout: Vec::new(), stderr: stderr.as_bytes().to_vec() };
        assert_eq!(Failure::of(&Error::Cancelled), Failure::Stopped);
        assert_eq!(Failure::of(&Error::Clone(CloneFailure::SizeLimit)), Failure::Limit);
        assert_eq!(Failure::of(&git("fatal: Authentication failed for 'https://x'")), Failure::Refused);
        assert_eq!(Failure::of(&git("fatal: Could not resolve host: github.com")), Failure::Offline);
        assert!(matches!(Failure::of(&git("PRIVATE_SOMETHING_ELSE")), Failure::Other(_)));
        for failure in [Failure::Limit, Failure::Refused, Failure::NotFound, Failure::Offline, Failure::Stopped] {
            assert!(!failure.code().is_empty());
        }
    }

    /// One frame of `draw` in a 900×600 window; the controls it registered.
    fn frame(mut draw: impl FnMut(&mut Ui, &mut HashMap<String, UiControl>)) -> HashMap<String, UiControl> {
        let ctx = Context::default();
        crate::style::install(&ctx);
        let mut widgets = HashMap::new();
        let bounds = Rect::from_min_size(Pos2::ZERO, vec2(900.0, 600.0));
        ctx.run_ui(RawInput { screen_rect: Some(bounds), ..Default::default() }, |ui| draw(ui, &mut widgets))
            .drop_without_applying_deltas();
        widgets
    }

    fn downloads(status: DownloadStatus) -> Downloads {
        Downloads { status, host: "github.com".into(), branch: Some("main".into()) }
    }

    #[test]
    fn views_show_downloads_while_they_run_and_failures_after() {
        let running = downloads(DownloadStatus { running: true, files: 5, received: 3, size: 90_000, downloaded: 34 * 1024, failure: None });
        let mut drawn = None;
        let widgets = frame(|ui, w| drawn = downloading(ui, Some(&running), "Downloading the changed files", "diff-cancel", w));
        assert_eq!(drawn, Some(false), "drawn, Stop not clicked");
        assert!(widgets.get("diff-cancel").is_some_and(|c| c.enabled));
        // Nothing downloading or no preview: the view's own loading state.
        let idle = downloads(DownloadStatus::default());
        assert!(frame(|ui, w| assert_eq!(downloading(ui, Some(&idle), "x", "diff-cancel", w), None)).is_empty());
        assert!(frame(|ui, w| assert_eq!(downloading(ui, None, "x", "diff-cancel", w), None)).is_empty());
        // A failed download: what still works, and Try again; never while one runs.
        let failed = downloads(DownloadStatus { failure: Some(FetchFailure::ConnectionFailed), ..Default::default() });
        let mut retry = None;
        let widgets = frame(|ui, w| retry = not_downloaded(ui, Some(&failed), "diff-retry", w));
        assert_eq!(retry, Some(false));
        assert!(widgets.contains_key("diff-retry"));
        assert!(frame(|ui, w| assert_eq!(not_downloaded(ui, Some(&idle), "diff-retry", w), None)).is_empty());
        let both = downloads(DownloadStatus { running: true, failure: Some(FetchFailure::ConnectionFailed), ..Default::default() });
        assert!(frame(|ui, w| assert_eq!(not_downloaded(ui, Some(&both), "diff-retry", w), None)).is_empty());
    }
}

/// What a read-only preview tab refuses: anything that changes a repository, runs
/// a command or tool in it, or opens a page that does. Browsing intents pass.
pub(crate) fn refused(intent: &Intent) -> bool {
    match intent {
        Intent::Tool(_)
        | Intent::Conflict(_)
        | Intent::Stash(_)
        | Intent::BranchReview(_)
        | Intent::RunContextCommand { .. }
        | Intent::Extension(_)
        | Intent::ExtensionCommand(_)
        | Intent::KeepLocal { .. }
        | Intent::BranchWorkflow(_)
        | Intent::MergeBranch(_)
        | Intent::RebaseOnto(_)
        | Intent::ExportSeries(_)
        | Intent::Drop(_)
        | Intent::ArrangeCommits(_)
        | Intent::SelectChange(..)
        | Intent::ConflictSide(_)
        | Intent::Terminal
        | Intent::CommitPush
        | Intent::SavedSwitch(_)
        | Intent::TerminalFor(_)
        | Intent::ShowFolder(_)
        | Intent::ImportPatch(_)
        | Intent::PastePatch
        | Intent::Execute(..)
        // A review window would outlive the preview and its downloads.
        | Intent::OpenReview(_)
        // Actions can re-run jobs on the host.
        | Intent::Actions(_)
        | Intent::GuidedSetup => true,
        Intent::View(view) => matches!(
            view,
            View::Changes | View::Stashes | View::Conflict | View::BranchReview | View::Summary | View::Actions
        ),
        Intent::Dialog(kind) => !matches!(
            kind,
            DialogKind::Explore
                | DialogKind::KeepPreview(_)
                | DialogKind::Clone
                | DialogKind::Open
                | DialogKind::NewRepository
                | DialogKind::RepositoryManager
                | DialogKind::Tags
                | DialogKind::GitHub
                | DialogKind::Statistics
                | DialogKind::Preferences
                | DialogKind::About
                | DialogKind::Legal
                | DialogKind::Changelog
                | DialogKind::CloseTab(_)
                | DialogKind::Quit
        ),
        _ => false,
    }
}

#[cfg(test)]
mod refusal_tests {
    use super::*;

    #[test]
    fn previews_refuse_changes_and_commands_but_allow_browsing() {
        for intent in [
            Intent::Terminal,
            Intent::Dialog(DialogKind::Push),
            Intent::Dialog(DialogKind::NewBranch),
            Intent::Dialog(DialogKind::Stash),
            Intent::Dialog(DialogKind::Integrations),
            Intent::View(View::Changes),
            Intent::View(View::Actions),
            Intent::MergeBranch("main".into()),
            Intent::CommitPush,
            Intent::OpenReview(crate::reviews::Request { commit: ObjectId::parse(&"1".repeat(40)).unwrap(), path: None, blame: false }),
        ] {
            assert!(refused(&intent));
        }
        for intent in [
            Intent::View(View::Commits),
            Intent::View(View::Tree),
            Intent::View(View::Map),
            Intent::Dialog(DialogKind::KeepPreview(1)),
            Intent::Dialog(DialogKind::Tags),
            Intent::Dialog(DialogKind::Statistics),
            Intent::Explore(None),
            Intent::Refresh,
            Intent::CloseTab(1),
        ] {
            assert!(!refused(&intent));
        }
    }

    #[test]
    fn previews_refuse_changed_code_searches_they_cannot_answer() {
        crate::test_common::isolated(
            "ROCKET_PREVIEW_UI_CASE",
            "preview_tab::refusal_tests::previews_refuse_changed_code_searches_they_cannot_answer",
            || {
                let fixture = crate::test_common::Fixture::new();
                let ctx = Context::default();
                let mut app = Rocket::with_storage(&ctx, vec![], fixture.root.join("logs"), fixture.root.join("state"));
                let cache = PreviewCache::open(fixture.root.join("previews")).unwrap();
                let slot = Arc::new(cache.reserve().unwrap());
                let mut tab = Tab::new(slot.repository(), None);
                tab.preview = Some(Box::new(Preview {
                    request: PreviewRequest {
                        source: CloneSource::Local(fixture.repository.clone()),
                        url: fixture.repository.display().to_string(),
                        name: "fixture".into(),
                        host: "this computer".into(),
                        web: None,
                        size_kb: None,
                        branch: None,
                    },
                    slot,
                    meter: PreviewDownloads::new(None),
                    unlimited: false,
                    state: PreviewState::Ready { branch: Some("main".into()) },
                    signing: None,
                    sign_in_error: None,
                }));
                app.tabs.push(tab);
                app.active = app.tabs.len() - 1;
                // The refused card: Sign in for an https address, Try again for others.
                for (url, sign_in) in [("https://github.com/acme/tool.git", true), ("ssh://git@github.com/acme/tool.git", false)] {
                    let preview = app.tabs.last_mut().unwrap().preview.as_mut().unwrap();
                    preview.request.url = url.into();
                    let preview = app.tabs.last().unwrap().preview.as_deref().unwrap();
                    let mut widgets = HashMap::new();
                    let bounds = Rect::from_min_size(Pos2::ZERO, vec2(900.0, 600.0));
                    ctx.run_ui(RawInput { screen_rect: Some(bounds), ..Default::default() }, |ui| {
                        failed_card(ui, preview, &Failure::Refused, true, &mut widgets, &mut Card::default());
                    })
                    .drop_without_applying_deltas();
                    assert_eq!(widgets.contains_key("preview-sign-in"), sign_in, "{url}");
                    assert_eq!(widgets.contains_key("preview-retry"), !sign_in, "{url}");
                }
                app.dispatch(Intent::FindInCommits(b"src/lib.rs".to_vec()));
                assert!(app.notice.as_deref().is_some_and(|n| n.starts_with("Changed code search needs every file version")));
                let tab = &app.tabs[app.active];
                assert_eq!(tab.search_kind, HistorySearchKind::Text);
                assert!(tab.search.is_empty());
            },
        );
    }
}
