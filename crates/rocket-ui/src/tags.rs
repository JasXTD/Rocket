//! Tag browser and reviewed local mutations. Git never runs on the frame thread.
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
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
mod remote;
pub use remote::{RemoteState, Show};

pub struct DialogState {
    pub remote: RemoteState,
    pub page: Option<TagPage>,
    pub selected: Option<TagEntry>,
    pub details: Option<TagDetails>,
    pub action: Option<TagAction>,
    pub preview: Option<Arc<TagPreview>>,
    pub list_task: Option<ReadTask<TagPage>>,
    pub detail_task: Option<ReadTask<TagDetails>>,
    pub task: Option<ReadTask<TagPreview>>,
    pub target: String,
    pub message: String,
    pub annotated: bool,
    pub sign: bool,
    pub limit: usize,
    pub applying: bool,
    pub(crate) read_list: bool,
    /// "Select several": the tags chosen and their review.
    pub batch: Option<crate::batch::TagBatch<TagPreview>>,
    /// Shows only tags whose name contains this, ignoring case.
    pub filter: String,
    pub order: TagOrder,
    /// The selected tag's annotation, read once when its details arrive.
    pub annotation: Option<(ObjectId, TagAnnotation)>,
    /// Whether the details show the raw tag object.
    pub raw: bool,
}
impl Default for DialogState {
    fn default() -> Self {
        Self {
            remote: RemoteState::default(),
            page: None,
            selected: None,
            details: None,
            action: None,
            preview: None,
            list_task: None,
            detail_task: None,
            task: None,
            target: "HEAD".into(),
            message: String::new(),
            annotated: true,
            sign: false,
            limit: 200,
            applying: false,
            read_list: true,
            batch: None,
            filter: String::new(),
            order: TagOrder::Newest,
            annotation: None,
            raw: false,
        }
    }
}
impl DialogState {
    pub fn running(&self) -> bool {
        self.list_task.is_some()
            || self.detail_task.is_some()
            || self.task.is_some()
            || self.remote.running()
            || self
                .batch
                .as_ref()
                .is_some_and(crate::batch::TagBatch::running)
    }
    pub fn reviewing(&self) -> bool {
        self.action.is_some()
            || self.applying
            || (self.remote.active && self.remote.action.is_some())
            || self.batch_open()
    }
    fn invalidate(&mut self) {
        self.task = None;
        self.preview = None;
    }
    /// Show the remote tab, with its last read. `selected` names a tag to select once
    /// the remote is read.
    fn open_remote(&mut self, selected: Option<Vec<u8>>) {
        self.list_task = None;
        self.detail_task = None;
        self.invalidate();
        self.remote.active = true;
        if selected.is_some() {
            // The named tag has to be in the list once it is read.
            self.remote.filter.clear();
            self.remote.show = Show::All;
            self.remote.selected = selected;
        }
        if self.remote.snapshot.is_some() {
            if self.remote_read_outdated() {
                self.remote.stale = true;
                self.remote.notice = Some(crate::i18n::tr("Tags changed here since this read.").into());
            } else if self.remote.stale && self.remote.notice.is_none() {
                self.remote.notice = Some(crate::i18n::tr(remote::OUT_OF_DATE).into());
            }
        }
    }
    /// Whether the local tags listed now differ from those the remote read saw. A list
    /// cut at its limit can miss a deleted tag; the review checks again.
    fn remote_read_outdated(&self) -> bool {
        let (Some(page), Some(snapshot)) = (&self.page, &self.remote.snapshot) else {
            return false;
        };
        let read: HashMap<&[u8], &ObjectId> = snapshot
            .entries
            .iter()
            .filter_map(|e| e.local.as_ref().map(|l| (e.name.as_slice(), &l.id)))
            .collect();
        (!page.more && page.entries.len() != read.len())
            || page.entries.iter().any(|e| read.get(e.name.as_slice()) != Some(&&e.id))
    }
    /// The annotation of the tag whose details show.
    fn shown_annotation(&self) -> Option<&TagAnnotation> {
        let id = &self.details.as_ref()?.entry.id;
        self.annotation
            .as_ref()
            .filter(|(of, _)| of == id)
            .map(|(_, parts)| parts)
    }
}
impl Rocket {
    pub(crate) fn start_tag_read(&mut self) {
        if self.remote_tags_active() {
            self.start_remote_tag_read();
            return;
        }
        let Some(d) = &mut self.dialog else { return };
        if !matches!(d.kind, DialogKind::Tags) {
            return;
        }
        let s = d.tags.get_or_insert_with(Default::default);
        if s.applying {
            return;
        }
        d.error = None;
        s.list_task = None;
        s.detail_task = None;
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
        match s.action {
            Some(TagAction::Create) => {
                let (name, target, message) = (
                    d.first.clone(),
                    s.target.clone(),
                    s.annotated.then(|| s.message.clone()),
                );
                let sign = s.sign && s.annotated;
                match ReadTask::spawn(move |cancel| {
                    if sign {
                        backend.create_signed_tag_preview(
                            &path,
                            &name,
                            &target,
                            message.as_deref().unwrap_or_default(),
                            &cancel,
                        )
                    } else {
                        backend.create_tag_preview(
                            &path,
                            &name,
                            &target,
                            message.as_deref(),
                            &cancel,
                        )
                    }
                }) {
                    Ok(task) => s.task = Some(task),
                    Err(e) => d.error = Some(user_error(&e)),
                }
            }
            Some(TagAction::Delete) => {
                let Some(entry) = s.selected.clone() else {
                    return;
                };
                match ReadTask::spawn(move |cancel| {
                    backend.delete_tag_preview(&path, &entry, &cancel)
                }) {
                    Ok(task) => s.task = Some(task),
                    Err(e) => d.error = Some(user_error(&e)),
                }
            }
            None if s.read_list => {
                let (limit, order) = (s.limit, s.order);
                s.details = None;
                match ReadTask::spawn(move |cancel| {
                    backend.tags_ordered(&path, limit, order, &cancel)
                }) {
                    Ok(task) => s.list_task = Some(task),
                    Err(e) => d.error = Some(user_error(&e)),
                }
            }
            None => {
                s.details = None;
                let Some(entry) = s.selected.clone() else {
                    return;
                };
                match ReadTask::spawn(move |cancel| backend.inspect_tag(&path, &entry, &cancel)) {
                    Ok(task) => s.detail_task = Some(task),
                    Err(e) => d.error = Some(user_error(&e)),
                }
            }
        }
    }
    pub(crate) fn poll_tag_read(&mut self) {
        self.poll_tag_batch();
        if self.remote_tags_active() {
            self.poll_remote_tag_read();
            return;
        }
        let Some(d) = &mut self.dialog else { return };
        let Some(s) = &mut d.tags else { return };
        let mut inspect = false;
        if let Some(result) = s.list_task.as_mut().and_then(ReadTask::try_finish) {
            s.list_task = None;
            match result {
                Ok(page) => {
                    // Tag names identify rows, even when several tags point at one commit.
                    s.selected = s
                        .selected
                        .as_ref()
                        .and_then(|old| page.entries.iter().find(|e| e.name == old.name))
                        .cloned();
                    s.page = Some(page);
                    s.read_list = false;
                    inspect = s.selected.is_some();
                }
                Err(e) => {
                    s.page = None;
                    s.selected = None;
                    d.error = Some(user_error(&e));
                }
            }
        }
        if let Some(result) = s.detail_task.as_mut().and_then(ReadTask::try_finish) {
            s.detail_task = None;
            match result {
                Ok(details) => {
                    s.annotation = details
                        .annotation
                        .as_deref()
                        .map(|raw| (details.entry.id.clone(), TagAnnotation::parse(raw)));
                    s.details = Some(details);
                }
                Err(e) => d.error = Some(user_error(&e)),
            }
        }
        if let Some(result) = s.task.as_mut().and_then(ReadTask::try_finish) {
            s.task = None;
            match result {
                Ok(preview) => s.preview = Some(Arc::new(preview)),
                Err(e) => d.error = Some(user_error(&e)),
            }
        }
        if inspect {
            self.start_tag_read();
        }
    }
    pub(crate) fn finish_tag_dialog(&mut self, owner: Option<u64>, outcome: &Outcome) {
        if self.finish_tag_batch(owner, outcome) {
            return;
        }
        if self.remote_tags_active() {
            self.finish_remote_tag_dialog(owner, outcome);
            return;
        }
        let Some(d) = &mut self.dialog else { return };
        let Some(s) = &mut d.tags else { return };
        if !s.applying || d.owner != owner {
            return;
        }
        s.applying = false;
        if outcome.command_succeeded
            && !outcome.recovery_pending
            && let Some(preview) = &s.preview
            && preview.action() == TagAction::Create
        {
            s.selected = Some(preview.details().entry.clone());
        }
        s.invalidate();
        if outcome.command_succeeded && !outcome.recovery_pending {
            s.action = None;
            s.read_list = true;
            d.first.clear();
            s.message.clear();
            self.start_tag_read();
        } else {
            d.error = Some(crate::i18n::trf!(
                "{summary}\n{details}\nReturn to the list and refresh before retrying.",
                summary = outcome.summary, details = outcome.details.join("\n")
            ));
        }
    }
    /// The commit a new tag points to by default: the one selected in history.
    fn default_tag_target(&self) -> String {
        self.tab()
            .and_then(|t| t.selected_commit.as_ref())
            .map_or_else(|| "HEAD".into(), ToString::to_string)
    }
    /// Tags page, "This repository" tab: list and details. Create and delete open the
    /// review dialog.
    pub(crate) fn tag_page(&mut self, ui: &mut Ui) {
        if self.remote_tags_active() {
            self.remote_tag_page(ui);
            return;
        }
        let writable = self.can_write_repository();
        let default_target = self.default_tag_target();
        let Some(mut d) = self.dialog.take() else {
            return;
        };
        let remote = remote_name(&d);
        let has_remote = !d.remotes.is_empty();
        let s = d.tags.get_or_insert_with(Default::default);
        let reviewing = s.action.is_some() || s.applying || s.batch.is_some();
        let waiting = s.running() || s.applying;
        let mut f = Flags::default();
        let mut act = None;
        // Under a review dialog the page is inert and publishes no controls.
        let mut inert = HashMap::new();
        let widgets = if reviewing {
            &mut inert
        } else {
            &mut self.frame_widgets
        };
        ui.add_enabled_ui(!reviewing, |ui| {
            let top = top(
                ui,
                widgets,
                Top {
                    remote: false,
                    count: s.page.as_ref().map(|p| p.entries.len()),
                    remote_count: s.remote.snapshot.as_ref().map(|p| p.entries.len()),
                    remote_name: &remote,
                    can_close: !s.applying,
                    can_create: !waiting && writable,
                    can_refresh: Some(!waiting),
                    can_switch: !waiting,
                },
            );
            f.dismiss = top.close;
            if top.new_tag {
                s.action = Some(TagAction::Create);
                s.target = default_target.clone();
                s.invalidate();
                d.error = None;
            }
            if top.refresh {
                s.read_list = true;
                f.refresh = true;
            }
            if top.remote {
                s.open_remote(None);
                d.error = None;
            }
            let some = s.page.as_ref().is_some_and(|p| p.entries.iter().any(|e| !e.symbolic));
            pages::toolbar(ui, |ui| {
                let r = search_field(ui, &mut s.filter, crate::i18n::tr("Filter tags"), 280.0);
                control(widgets, "dialog-tags-filter", &r, ui.clip_rect());
                ui.add_enabled_ui(!waiting, |ui| {
                    let orders = [
                        ("dialog-tags-sort-newest", crate::i18n::tr("Newest"), TagOrder::Newest),
                        ("dialog-tags-sort-name", crate::i18n::tr("Name"), TagOrder::Name),
                    ];
                    let items = orders.map(|(_, label, order)| (label, s.order == order));
                    for (r, (key, _, order)) in pages::segments(ui, &items).iter().zip(orders) {
                        control(widgets, key, r, ui.clip_rect());
                        if r.clicked() && s.order != order {
                            s.order = order;
                            s.read_list = true;
                            f.refresh = true;
                        }
                    }
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let r = pages::ghost(ui, !waiting && writable && some, crate::i18n::tr("Select several"));
                    control(widgets, "dialog-tags-batch", &r, ui.clip_rect());
                    if r.clicked() {
                        s.batch = Some(Default::default());
                        d.error = None;
                    }
                });
            });
            if !reviewing && let Some(error) = &d.error {
                pages::toolbar(ui, |ui| pages::callout(ui, RED(), error));
            }
            let body = ui.available_rect_before_wrap();
            pages::split(ui, |ui, side| match side {
                pages::Side::List => f.refresh |= tag_list(ui, s, waiting, widgets),
                pages::Side::Details => {
                    let can = Can {
                        writable,
                        waiting,
                        error: d.error.is_some(),
                        remote: has_remote.then_some(remote.as_str()),
                    };
                    act = details_panel(ui, s, widgets, &can);
                }
            });
            pages::details_scroll(ui, widgets, !s.applying);
            pages::region(widgets, "dialog-tags-scroll", body.intersect(ui.clip_rect()), !s.applying);
        });
        let mut verify = None;
        let mut show = None;
        match act {
            Some(Act::Verify(entry)) => verify = Some(entry),
            Some(Act::Show(id)) => show = Some(id),
            Some(Act::Raw) => s.raw = !s.raw,
            Some(Act::Delete) => {
                s.action = Some(TagAction::Delete);
                f.refresh = true;
            }
            Some(Act::Publish(name)) => {
                s.open_remote(Some(name));
                d.error = None;
                f.refresh = true;
            }
            None => {}
        }
        self.finish_tag_frame(d, f);
        if let Some(entry) = verify {
            self.show_dialog(DialogKind::Signing(Some(SignatureTarget::Tag(entry))));
        }
        if let Some(id) = show {
            self.close_browsing_page();
            self.dispatch(Intent::SelectCommit(id));
        }
    }

    /// Create/delete review dialog over the Tags page. Also drops a page whose
    /// repository is no longer active.
    pub(crate) fn tag_dialog(&mut self, ctx: &Context) {
        if self
            .dialog
            .as_ref()
            .and_then(|d| d.tags.as_ref())
            .is_some_and(DialogState::batch_open)
        {
            self.tag_batch_dialog(ctx);
            return;
        }
        if self.remote_tags_active() {
            self.remote_tag_dialog(ctx);
            return;
        }
        let writable = self.can_write_repository();
        let Some(mut d) = self.dialog.take() else {
            return;
        };
        let remote = remote_name(&d);
        let s = d.tags.get_or_insert_with(Default::default);
        // Global repository shortcuts can change tabs while a page is open.
        // Drop a read/review belonging to the old tab instead of presenting it in
        // the new repository. An already-running write retains its original owner.
        if d.owner != self.tab().map(|tab| tab.id) && !s.applying {
            return;
        }
        if s.action.is_none() && !s.applying {
            self.dialog = Some(d);
            return;
        }
        let waiting = s.running() || s.applying;
        let mut f = Flags::default();
        let create = s.action == Some(TagAction::Create);
        let name = s
            .preview
            .as_ref()
            .map(|p| display(&p.details().entry.name))
            .or_else(|| s.selected.as_ref().map(|e| display(&e.name)))
            .unwrap_or_default();
        let title = if create {
            crate::i18n::tr("Create tag").to_owned()
        } else {
            crate::i18n::trf!("Delete tag {name}?", name)
        };
        let head = pages::ReviewHeader {
            image: Icon::Tag,
            title: &title,
            subtitle: if create {
                crate::i18n::tr("Name a commit, such as a release. Existing tags are never replaced.")
            } else {
                crate::i18n::tr("Removes the tag from this repository. The commit it marks stays.")
            },
            danger: !create,
        };
        let running = pages::Running::of(self.write.as_ref());
        let response = pages::review(
            ctx,
            "rocket-tags",
            head,
            &mut self.frame_widgets,
            "dialog-tags-scroll",
            !s.applying,
            |ui, part, widgets| match part {
                pages::Part::Body => {
                    if create {
                        let mut edited = false;
                        let taken = s.page.as_ref().is_some_and(|p| {
                            p.entries.iter().any(|e| e.name == d.first.trim().as_bytes())
                        });
                        form_row(ui, crate::i18n::tr("Name"), |ui| {
                            let r = ui.add_enabled(
                                !s.applying,
                                TextEdit::singleline(&mut d.first)
                                    .hint_text("v1.2")
                                    .desired_width(f32::INFINITY)
                                    .min_size(vec2(0.0, 30.0))
                                    .align(Align2::LEFT_CENTER),
                            );
                            control(widgets, "dialog-tag-name", &r, ui.clip_rect());
                            edited |= r.changed();
                            if taken {
                                check(ui, false, crate::i18n::tr("A tag with this name already exists"));
                            } else if !d.first.trim().is_empty() {
                                check(ui, true, crate::i18n::tr("New tag name"));
                            }
                        });
                        form_row(ui, crate::i18n::tr("Points to"), |ui| {
                            let r = ui.add_enabled(
                                !s.applying,
                                TextEdit::singleline(&mut s.target)
                                    .font(TextStyle::Monospace)
                                    .desired_width(f32::INFINITY)
                                    .min_size(vec2(0.0, 30.0))
                                    .align(Align2::LEFT_CENTER),
                            );
                            control(widgets, "dialog-tag-target", &r, ui.clip_rect());
                            edited |= r.changed();
                            match s.preview.as_ref().map(|p| p.details()) {
                                Some(details) => commit_line(ui, &details.target, details.commit.as_ref(), true),
                                None => pages::note(ui, crate::i18n::tr("A branch, tag or commit ID; checked when you review.")),
                            }
                        });
                        form_row(ui, crate::i18n::tr("Type"), |ui| {
                            ui.add_enabled_ui(!s.applying, |ui| {
                                let kinds = [(false, crate::i18n::tr("Lightweight"), "light"), (true, crate::i18n::tr("Annotated"), "annotated")];
                                let items = kinds.map(|(value, label, _)| (label, s.annotated == value));
                                for (r, (value, _, id)) in pages::segments(ui, &items).iter().zip(kinds) {
                                    control(widgets, format!("dialog-tag-{id}"), r, ui.clip_rect());
                                    if r.clicked() && s.annotated != value {
                                        s.annotated = value;
                                        edited = true;
                                    }
                                }
                            });
                            pages::note(
                                ui,
                                if s.annotated {
                                    crate::i18n::tr("Also records who tagged it, when, and a message.")
                                } else {
                                    crate::i18n::tr("Only a name for the commit: no tagger, date or message.")
                                },
                            );
                        });
                        if s.annotated {
                            form_row(ui, crate::i18n::tr("Message"), |ui| {
                                let r = ui.add_enabled(
                                    !s.applying,
                                    TextEdit::multiline(&mut s.message)
                                        .desired_rows(3)
                                        .desired_width(f32::INFINITY),
                                );
                                control(widgets, "dialog-tag-message", &r, ui.clip_rect());
                                edited |= r.changed();
                            });
                            form_row(ui, "", |ui| {
                                ui.horizontal(|ui| {
                                    let r = ui.add_enabled(
                                        !s.applying,
                                        checkbox(&mut s.sign, crate::i18n::tr("Sign with your signing key")),
                                    );
                                    control(widgets, "dialog-tag-sign", &r, ui.clip_rect());
                                    edited |= r.changed();
                                    ui.label(
                                        RichText::new(crate::i18n::tr("· from Repository settings"))
                                            .size(12.0)
                                            .color(FAINT()),
                                    );
                                });
                            });
                        }
                        if edited {
                            s.invalidate();
                            d.error = None;
                        }
                        if let Some(p) = &s.preview {
                            ui.add_space(4.0);
                            creates_card(ui, p);
                        }
                    } else if let Some(p) = &s.preview {
                        deletes_card(ui, p);
                        pages::callout(
                            ui,
                            AMBER(),
                            &crate::i18n::trf!(
                                "Only this repository's tag is deleted. If a remote such as {remote} has {name}, the next fetch brings it back.", remote, name
                            ),
                        );
                    }
                    // A running tag operation shows its status and Stop in the footer.
                    if waiting && !s.applying {
                        pages::waiting(ui, crate::i18n::tr("Reading tags and the reviewed object…"));
                    }
                    if let Some(error) = &d.error {
                        pages::callout(ui, RED(), error);
                    }
                }
                pages::Part::Footer if s.applying => {
                    f.cancel = pages::running_footer(ui, widgets, &running, &[]);
                }
                pages::Part::Footer => {
                    let ready = !waiting && writable && s.preview.is_some() && d.error.is_none();
                    let r = if create {
                        pages::primary(ui, ready, crate::i18n::tr("Create tag"))
                    } else {
                        pages::danger(ui, ready, crate::i18n::tr("Delete tag"))
                    };
                    control(widgets, "dialog-submit", &r, ui.clip_rect());
                    f.submit = r.clicked();
                    let r = if s.preview.is_some() || d.error.is_some() || !create {
                        ui.add_enabled(!waiting, Button::new(crate::i18n::tr("Review again")))
                    } else {
                        pages::primary(ui, !waiting, crate::i18n::tr("Review"))
                    };
                    control(widgets, "dialog-preview-refresh", &r, ui.clip_rect());
                    f.refresh |= r.clicked();
                    pages::footer_status(ui, false, review_status(waiting, d.error.is_some(), s.preview.is_some()));
                }
            },
        );
        // The × returns to the Tags page; automation's old Cancel id is that ×.
        if let Some(close) = self.frame_widgets.get("dialog-close").copied() {
            self.frame_widgets.insert("dialog-tag-back".into(), close);
        }
        f.back |= response.should_close() && !s.applying;
        self.finish_tag_frame(d, f);
    }

    /// Apply one frame's page or review choices.
    fn finish_tag_frame(&mut self, mut d: Dialog, mut f: Flags) {
        let s = d.tags.get_or_insert_with(Default::default);
        if f.back {
            s.action = None;
            s.invalidate();
            s.detail_task = None;
            s.read_list = true;
            d.error = None;
            f.refresh = true;
        }
        if f.cancel {
            self.cancel_write();
        }
        if f.submit
            && let (Some(owner), Some(preview)) = (d.owner, s.preview.clone())
        {
            s.applying = true;
            self.execute(owner, Action::Tag(preview));
            if self.write.is_none() {
                s.applying = false;
                d.error =
                    Some(crate::i18n::tr("Could not start. Wait for the repository, then review again.").into());
            }
        }
        if !f.dismiss {
            self.dialog = Some(d);
        }
        if f.refresh {
            self.start_tag_read();
        }
    }
}

#[derive(Default)]
struct Flags {
    dismiss: bool,
    refresh: bool,
    submit: bool,
    back: bool,
    cancel: bool,
}

/// The footer line of a review: where it stands.
pub(crate) fn review_status(waiting: bool, error: bool, reviewed: bool) -> &'static str {
    if waiting {
        crate::i18n::tr("Reviewing")
    } else if error {
        crate::i18n::tr("Fix the problem above, then review again")
    } else if reviewed {
        crate::i18n::tr("Reviewed · nothing has changed yet")
    } else {
        crate::i18n::tr("Review to see exactly what will happen")
    }
}

/// The selected remote's name, as the Tags page names it ("On origin").
pub(crate) fn remote_name(d: &Dialog) -> String {
    d.remotes
        .get(d.remote)
        .map_or_else(|| crate::i18n::tr("a remote").into(), |r| display(r))
}

/// What the Tags page's title row and tabs show; both tabs share them.
struct Top<'a> {
    remote: bool,
    count: Option<usize>,
    /// Tags in the last read of the remote.
    remote_count: Option<usize>,
    remote_name: &'a str,
    can_close: bool,
    can_create: bool,
    /// `None` hides Refresh (the remote tab reads only when asked).
    can_refresh: Option<bool>,
    can_switch: bool,
}
/// What the title row and tabs were asked to do this frame.
#[derive(Default)]
struct TopClicks {
    close: bool,
    new_tag: bool,
    refresh: bool,
    /// Switch to "This repository".
    local: bool,
    /// Switch to the remote tab.
    remote: bool,
}
fn top(ui: &mut Ui, widgets: &mut HashMap<String, UiControl>, t: Top) -> TopClicks {
    let mut out = TopClicks::default();
    pages::header(
        ui,
        Icon::Tag,
        crate::i18n::tr("Tags"),
        crate::i18n::tr("Names for important commits, such as releases. Shared by this repository's worktrees."),
        |ui| {
            let r = pages::ghost(ui, t.can_close, crate::i18n::tr("Close"));
            control(widgets, "dialog-cancel", &r, ui.clip_rect());
            out.close = r.clicked();
            let r = pages::primary(ui, t.can_create, crate::i18n::tr("New tag"));
            control(widgets, "dialog-tag-new", &r, ui.clip_rect());
            out.new_tag = r.clicked();
            if let Some(enabled) = t.can_refresh {
                let r = crate::chrome::icon_button(ui, Icon::Refresh, enabled, false, 32.0)
                    .on_hover_text(crate::i18n::tr("Refresh"));
                control(widgets, "dialog-preview-refresh", &r, ui.clip_rect());
                out.refresh = r.clicked();
            }
        },
    );
    let remote = crate::i18n::trf!("On {remote}", remote = t.remote_name);
    let picked = pages::tabs(
        ui,
        &[
            pages::TabItem {
                label: crate::i18n::tr("This repository"),
                count: t.count,
                selected: !t.remote,
                enabled: t.can_switch,
                key: "dialog-tags-local",
            },
            pages::TabItem {
                label: &remote,
                count: t.remote_count,
                selected: t.remote,
                enabled: t.can_switch,
                key: "dialog-tags-remote",
            },
        ],
        widgets,
    );
    out.local = picked == Some(0) && t.remote;
    out.remote = picked == Some(1) && !t.remote;
    out
}

/// What a page holds for a tag of this name.
pub(crate) fn summary_in<'a>(page: Option<&'a TagPage>, name: &[u8]) -> Option<&'a TagSummary> {
    let page = page?;
    let i = page.entries.iter().position(|e| e.name == name)?;
    page.summaries.get(i)
}

/// A tag's kind, as its pill shows it.
pub(crate) fn kind(entry: &TagEntry) -> (&'static str, Color32) {
    if entry.symbolic {
        (crate::i18n::tr("Symbolic"), pages::NEUTRAL())
    } else if entry.annotated() {
        (crate::i18n::tr("Annotated"), pages::INFO())
    } else {
        (crate::i18n::tr("Lightweight"), pages::NEUTRAL())
    }
}

pub(crate) fn short(id: &ObjectId) -> &str {
    &id.as_str()[..7]
}

/// "3 h ago" for a Unix time.
pub(crate) fn age(unix: i64) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    ago(now.saturating_sub(unix).max(0) as u64, unix)
}

/// One tag in a list: its name and kind, then the commit it names; `meta` on the right.
pub(crate) struct TagRow<'a> {
    pub name: &'a str,
    /// What the tile shows, when it says more than that this is a tag.
    pub tile: Option<(Icon, Color32)>,
    pub kind: Option<(&'a str, Color32)>,
    pub sha: &'a str,
    pub subject: &'a str,
    pub meta: &'a str,
    pub meta_color: Color32,
}
pub(crate) fn tag_row(ui: &mut Ui, row: TagRow, selected: bool, enabled: bool) -> Response {
    let response = ui.add_enabled(
        enabled,
        Button::new("")
            .frame(false)
            .min_size(vec2(ui.available_width(), 50.0)),
    );
    let rect = response.rect;
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
    let tile = Rect::from_min_size(pos2(rect.left() + 12.0, rect.center().y - 15.0), vec2(30.0, 30.0));
    painter.rect_filled(tile, 7.0, CARD());
    let (mark, mark_color) = row.tile.unwrap_or((Icon::Tag, if selected { BLUE() } else { HINT() }));
    icon(painter, tile.shrink(8.0), mark, mark_color);
    let right = rect.right() - 12.0;
    let meta = painter.layout_no_wrap(row.meta.to_owned(), FontId::proportional(12.0), row.meta_color);
    let meta_width = meta.size().x;
    painter.galley(
        pos2(right - meta_width, rect.center().y - meta.size().y / 2.0),
        meta,
        row.meta_color,
    );
    let left = tile.right() + 12.0;
    let width = (right - meta_width - 16.0 - left).max(40.0);
    let ink = if enabled { TEXT() } else { MUTED() };
    // The name, then its kind pill right after it.
    let pill = row.kind.map(|(kind, color)| {
        let galley = painter.layout_no_wrap(kind.to_owned(), FontId::proportional(11.5), color);
        (galley, color)
    });
    let pill_width = pill.as_ref().map_or(0.0, |(galley, _)| galley.size().x + 22.0);
    let mut job = text::LayoutJob::simple_singleline(row.name.into(), semibold(13.0), ink);
    job.wrap = text::TextWrapping::truncate_at_width((width - pill_width).max(20.0));
    let name = painter.layout_job(job);
    let line = rect.center().y - 9.0;
    let after_name = left + name.size().x + 8.0;
    painter.galley(pos2(left, line - name.size().y / 2.0), name, ink);
    if let Some((pill, color)) = pill {
        let pill_rect = Rect::from_min_size(pos2(after_name, line - 9.0), vec2(pill_width - 8.0, 18.0));
        painter.rect_filled(pill_rect, 9.0, color.gamma_multiply(0.14));
        painter.galley(
            pos2(pill_rect.left() + 7.0, pill_rect.center().y - pill.size().y / 2.0),
            pill,
            color,
        );
    }
    // The commit: its short ID, then its subject.
    let mut job = text::LayoutJob::default();
    job.append(row.sha, 0.0, TextFormat::simple(FontId::monospace(11.5), MUTED()));
    if !row.subject.is_empty() {
        let lead = if row.sha.is_empty() { "" } else { " · " };
        job.append(
            &format!("{lead}{}", row.subject),
            0.0,
            TextFormat::simple(FontId::proportional(12.0), HINT()),
        );
    }
    job.wrap = text::TextWrapping::truncate_at_width(width);
    let commit = painter.layout_job(job);
    painter.galley(
        pos2(left, rect.center().y + 9.0 - commit.size().y / 2.0),
        commit,
        HINT(),
    );
    response.widget_info(|| WidgetInfo::selected(Role::Button, enabled, selected, row.name));
    response
}

/// The list's tags that match the filter, by index into the page.
fn visible_rows(s: &DialogState) -> Vec<usize> {
    let Some(page) = &s.page else {
        return Vec::new();
    };
    let needle = s.filter.trim().to_lowercase();
    page.entries
        .iter()
        .enumerate()
        .filter(|(_, e)| needle.is_empty() || display(&e.name).to_lowercase().contains(&needle))
        .map(|(i, _)| i)
        .collect()
}

fn tag_list(
    ui: &mut Ui,
    s: &mut DialogState,
    waiting: bool,
    widgets: &mut HashMap<String, UiControl>,
) -> bool {
    let mut changed = false;
    let rows = visible_rows(s);
    let height = (ui.available_height() - 40.0).max(80.0);
    let list = ScrollArea::vertical()
        .id_salt("tag-list")
        .max_height(height)
        .auto_shrink([false, true])
        .show_rows(ui, 50.0, rows.len(), |ui, range| {
            ui.spacing_mut().item_spacing.y = 2.0;
            let Some(page) = &s.page else { return };
            for &i in &rows[range] {
                let entry = &page.entries[i];
                let summary = page.summaries.get(i);
                let name = display(&entry.name);
                let subject = summary.map_or_else(String::new, |m| display(&m.subject));
                let meta = summary.and_then(|m| m.date).map(age).unwrap_or_default();
                let selected = s.selected.as_ref().is_some_and(|e| e.name == entry.name);
                let r = tag_row(
                    ui,
                    TagRow {
                        name: &name,
                        tile: None,
                        kind: Some(kind(entry)),
                        sha: summary
                            .and_then(|m| m.target.as_ref())
                            .map_or("", short),
                        subject: &subject,
                        meta: &meta,
                        meta_color: HINT(),
                    },
                    selected,
                    !waiting,
                );
                control(widgets, format!("dialog-tag-row-{i}"), &r, ui.clip_rect());
                if r.clicked() {
                    s.selected = Some(entry.clone());
                    s.read_list = false;
                    s.details = None;
                    s.raw = false;
                    changed = true;
                }
            }
        });
    pages::region(
        widgets,
        "dialog-tag-list-scroll",
        list.inner_rect.intersect(ui.clip_rect()),
        !waiting,
    );
    match &s.page {
        Some(page) if page.entries.is_empty() => {
            pages::empty(ui, crate::i18n::tr("No tags yet. New tag names a commit, such as a release."));
        }
        Some(page) => {
            if rows.is_empty() {
                pages::empty(ui, crate::i18n::tr("No tags match the filter."));
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let total = page.entries.len();
                let order = match s.order {
                    TagOrder::Newest => crate::i18n::tr("newest first"),
                    TagOrder::Name => crate::i18n::tr("by name"),
                };
                let count = if rows.len() < total {
                    crate::i18n::trf!("Tags: {shown} of {total}", shown = rows.len(), total)
                } else {
                    crate::i18n::trf!("Tags: {count}", count = total)
                };
                ui.label(RichText::new(format!("{count} · {order}")).size(12.0).color(FAINT()));
                if page.more {
                    let r = pages::ghost(ui, !waiting && s.limit < 2000, crate::i18n::tr("Show more"));
                    control(widgets, "dialog-tags-more", &r, ui.clip_rect());
                    if r.clicked() {
                        s.limit = (s.limit + 200).min(2000);
                        s.read_list = true;
                        changed = true;
                    }
                }
            });
            if page.more && s.limit == 2000 {
                pages::callout(ui, AMBER(), crate::i18n::tr("Showing the first 2,000 tags."));
            }
        }
        None if waiting => pages::loading(ui, crate::i18n::tr("Reading tags…")),
        None => {}
    }
    changed
}

/// What the details card was asked to do.
enum Act {
    Verify(TagEntry),
    Show(ObjectId),
    Raw,
    Delete,
    Publish(Vec<u8>),
}
/// What the details card's actions may do now.
struct Can<'a> {
    writable: bool,
    waiting: bool,
    error: bool,
    /// The remote a tag can be published to, if there is one.
    remote: Option<&'a str>,
}

fn details_panel(
    ui: &mut Ui,
    s: &DialogState,
    widgets: &mut HashMap<String, UiControl>,
    can: &Can,
) -> Option<Act> {
    let Some(details) = &s.details else {
        if s.detail_task.is_some() {
            pages::loading(ui, crate::i18n::tr("Reading the tag…"));
        } else {
            pages::empty(ui, crate::i18n::tr("Select a tag to see the commit it marks and its message."));
        }
        return None;
    };
    let mut act = None;
    let annotation = s.shown_annotation();
    let signed = annotation.is_some_and(|a| a.signed);
    ui.spacing_mut().item_spacing.y = 10.0;
    ui.add(Label::new(RichText::new(display(&details.entry.name)).font(semibold(18.0))).wrap());
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let (kind, color) = kind(&details.entry);
        pages::pill(ui, kind, color);
        if signed {
            pages::pill(ui, crate::i18n::tr("Signed · not checked"), AMBER());
        } else {
            pages::pill(ui, crate::i18n::tr("Not signed"), pages::NEUTRAL());
        }
    });
    card(ui, crate::i18n::tr("Points to"), |ui| {
        if let Some(commit) = &details.commit {
            ui.add(Label::new(RichText::new(subject(commit)).strong()).wrap());
            commit_line(ui, &details.target, Some(commit), false);
            let r = ui.add(
                Button::new(RichText::new(crate::i18n::tr("Show in history")).size(12.5).color(BLUE())).frame(false),
            );
            control(widgets, "dialog-tag-show-commit", &r, ui.clip_rect());
            if r.clicked() {
                act = Some(Act::Show(details.target.clone()));
            }
        } else {
            ui.label(crate::i18n::trf!("A {kind} object, not a commit", kind = details.target_type));
            ui.label(RichText::new(details.target.as_str()).monospace().size(12.0).color(MUTED()));
        }
    });
    if let Some(parts) = annotation {
        card(ui, crate::i18n::tr("Message"), |ui| {
            let message = String::from_utf8_lossy(&parts.message);
            let message = message.trim_end();
            if message.is_empty() {
                pages::note(ui, crate::i18n::tr("No message."));
            } else {
                let (text, cut) = preview_text(message.as_bytes());
                ui.add(Label::new(text).wrap());
                if cut {
                    pages::note(ui, crate::i18n::tr("Showing the first 16,000 characters."));
                }
            }
            if let Some(tagger) = &parts.tagger {
                pages::note(
                    ui,
                    &crate::i18n::trf!(
                        "Tagged by {name} · {date}", name = display(&tagger.name),
                        date = date(tagger.timestamp, tagger.offset_minutes)
                    ),
                );
            }
        });
    }
    if let Some(raw) = &details.annotation {
        let r = pages::disclosure(ui, s.raw, crate::i18n::tr("Raw tag object"));
        control(widgets, "dialog-tag-raw", &r, ui.clip_rect());
        if r.clicked() {
            act = Some(Act::Raw);
        }
        if s.raw {
            let (text, truncated) = preview_text(raw);
            Frame::new()
                .fill(BG())
                .corner_radius(6)
                .inner_margin(8)
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.add(Label::new(RichText::new(text).monospace().size(12.0)).wrap());
                });
            if truncated {
                pages::callout(ui, AMBER(), crate::i18n::tr("Showing the first 16,000 characters."));
            }
        }
    } else if !details.entry.symbolic {
        pages::note(
            ui,
            crate::i18n::tr("A lightweight tag is only a name for this commit: it has no tagger, date or message."),
        );
    }
    pages::divider(ui);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let name = details.entry.name.clone();
        let label = crate::i18n::trf!("Publish to {remote}", remote = can.remote.unwrap_or(crate::i18n::tr("a remote")));
        let r = ui
            .add_enabled(
                !can.waiting && can.remote.is_some() && !details.entry.symbolic,
                Button::new(label),
            )
            .on_disabled_hover_text(crate::i18n::tr("Add a remote in Repository → Remotes & Hosting → Manage Remotes first."));
        control(widgets, "dialog-tag-publish", &r, ui.clip_rect());
        if r.clicked() {
            act = Some(Act::Publish(name));
        }
        // Any annotated tag: checking an unsigned one reports that it has no signature.
        if details.entry.annotated() {
            let r = ui.add_enabled(!can.waiting, Button::new(crate::i18n::tr("Verify signature")));
            control(widgets, "dialog-tag-verify", &r, ui.clip_rect());
            if r.clicked() {
                act = Some(Act::Verify(details.entry.clone()));
            }
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let r = pages::danger_ghost(
                ui,
                !can.waiting && can.writable && !can.error && !details.entry.symbolic,
                crate::i18n::tr("Delete tag"),
            );
            control(widgets, "dialog-tag-delete", &r, ui.clip_rect());
            if r.clicked() {
                act = Some(Act::Delete);
            }
        });
    });
    act
}

/// A bordered card with a small title strip.
pub(crate) fn card(ui: &mut Ui, title: &str, body: impl FnOnce(&mut Ui)) {
    pages::titled_card(
        ui,
        |ui| {
            ui.label(RichText::new(title).size(12.0).color(MUTED()).strong());
        },
        body,
    );
}

/// "1a2b3c4 · subject · author · date" (or "· 3 h ago" when `relative`).
pub(crate) fn commit_line(ui: &mut Ui, id: &ObjectId, commit: Option<&Commit>, relative: bool) {
    let plain = |text: &str, color| (text.to_owned(), TextFormat::simple(FontId::proportional(12.0), color));
    let mut job = text::LayoutJob::default();
    job.append(short(id), 0.0, TextFormat::simple(FontId::monospace(12.0), MUTED()));
    if let Some(commit) = commit {
        let when = if relative {
            age(commit.author.timestamp)
        } else {
            date(commit.author.timestamp, commit.author.offset_minutes)
        };
        let rest = if relative {
            format!(" · {} · {when}", subject(commit))
        } else {
            format!(" · {} · {when}", display(&commit.author.name))
        };
        let (text, format) = plain(&rest, HINT());
        job.append(&text, 0.0, format);
    }
    job.wrap = text::TextWrapping::truncate_at_width(ui.available_width());
    ui.label(job);
}

/// A form row: the label in a fixed column, the field and its notes beside it.
pub(crate) fn form_row(ui: &mut Ui, label: &str, content: impl FnOnce(&mut Ui)) {
    ui.horizontal_top(|ui| {
        let (rect, _) = ui.allocate_exact_size(vec2(96.0, 30.0), Sense::hover());
        ui.painter().text(
            rect.left_center(),
            Align2::LEFT_CENTER,
            label,
            FontId::proportional(13.0),
            MUTED(),
        );
        ui.vertical(|ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 5.0;
            content(ui)
        });
    });
}

/// A live check under a field: a green tick, or an amber warning.
pub(crate) fn check(ui: &mut Ui, ok: bool, text: &str) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 5.0;
        let color = if ok { GREEN() } else { AMBER() };
        let (mark, _) = ui.allocate_exact_size(vec2(12.0, 12.0), Sense::hover());
        icon(ui.painter(), mark, if ok { Icon::Check } else { Icon::Close }, color);
        ui.label(RichText::new(text).size(12.0).color(color));
    });
}

/// The first line of an annotated tag's message, for review rows.
fn first_line(parts: &TagAnnotation) -> String {
    let message = String::from_utf8_lossy(&parts.message);
    let line = message.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
    let mut line: String = line.chars().take(200).collect();
    if line.is_empty() {
        line = crate::i18n::tr("No message").into();
    }
    line
}

/// What creating the reviewed tag makes.
fn creates_card(ui: &mut Ui, p: &TagPreview) {
    let details = p.details();
    pages::titled_card(
        ui,
        |ui| {
            ui.label(RichText::new(crate::i18n::trf!("Creates {name}", name = display(&details.entry.name))).strong());
            let (kind, color) = kind(&details.entry);
            pages::pill(ui, kind, color);
        },
        |ui| {
            pages::key_value(ui, crate::i18n::tr("Points to"), 104.0, |ui| {
                commit_ref(ui, &details.target, details.commit.as_ref(), "");
            });
            if let Some(raw) = &details.annotation {
                let parts = TagAnnotation::parse(raw);
                if let Some(tagger) = &parts.tagger {
                    pages::key_value(ui, crate::i18n::tr("Tagged by"), 104.0, |ui| {
                        let mut job = text::LayoutJob::default();
                        job.append(&display(&tagger.name), 0.0, TextFormat::simple(FontId::proportional(13.0), TEXT()));
                        job.append(crate::i18n::tr(" · when you create it"), 0.0, TextFormat::simple(FontId::proportional(13.0), FAINT()));
                        ui.label(job);
                    });
                }
                pages::key_value(ui, crate::i18n::tr("Message"), 104.0, |ui| {
                    ui.add(Label::new(first_line(&parts)).truncate());
                });
                pages::key_value(ui, crate::i18n::tr("Signature"), 104.0, |ui| match p.signing() {
                    Some(signing) => {
                        ui.label(crate::i18n::trf!("Signed with your {format} key when created", format = signing.format()));
                    }
                    None => {
                        ui.label(RichText::new(crate::i18n::tr("Not signed")).color(MUTED()));
                    }
                });
            }
        },
    );
}

/// What deleting the reviewed tag removes, and what stays.
fn deletes_card(ui: &mut Ui, p: &TagPreview) {
    let details = p.details();
    pages::titled_card(
        ui,
        |ui| {
            ui.label(RichText::new(crate::i18n::trf!("Deletes {name}", name = display(&details.entry.name))).strong());
            let (kind, color) = kind(&details.entry);
            pages::pill(ui, kind, color);
        },
        |ui| {
            pages::key_value(ui, crate::i18n::tr("Points to"), 104.0, |ui| {
                commit_ref(ui, &details.target, details.commit.as_ref(), crate::i18n::tr(" · the commit stays"));
            });
            if let Some(raw) = &details.annotation {
                let parts = TagAnnotation::parse(raw);
                pages::key_value(ui, crate::i18n::tr("Message"), 104.0, |ui| {
                    ui.add(Label::new(first_line(&parts)).truncate());
                });
            }
            if let Some(reference) = p.recovery_reference() {
                pages::key_value(ui, crate::i18n::tr("Recovery"), 104.0, |ui| {
                    ui.label(crate::i18n::tr("Kept in refs/rocket/deleted-tags"))
                        .on_hover_text(reference);
                });
            }
        },
    );
}

/// "1a2b3c4 · subject" and a dimmed `tail`, cut short rather than wrapped.
pub(crate) fn commit_ref(ui: &mut Ui, id: &ObjectId, commit: Option<&Commit>, tail: &str) {
    let mut job = text::LayoutJob::default();
    job.append(short(id), 0.0, TextFormat::simple(FontId::monospace(12.5), TEXT()));
    if let Some(commit) = commit {
        job.append(
            &format!(" · {}", subject(commit)),
            0.0,
            TextFormat::simple(FontId::proportional(13.0), TEXT()),
        );
    }
    job.append(tail, 0.0, TextFormat::simple(FontId::proportional(13.0), FAINT()));
    job.wrap = text::TextWrapping::truncate_at_width(ui.available_width());
    ui.label(job);
}

/// Bound decoding as well as painting/serialization. A large existing tag must
/// not make every UI frame or MCP state request scan its entire object.
pub fn preview_text(bytes: &[u8]) -> (String, bool) {
    const CHARACTERS: usize = 16000;
    let prefix = crate::text::prefix(bytes, (CHARACTERS + 1) * 4);
    let decoded = String::from_utf8_lossy(prefix);
    let mut chars = decoded.chars();
    let text = chars.by_ref().take(CHARACTERS).collect();
    let truncated = prefix.len() < bytes.len() || chars.next().is_some();
    (text, truncated)
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
    fn large_inspection_text_preserves_unicode_and_marks_limits() {
        let input = "日本語 e\u{301} 🚀\n".repeat(100000);
        let (text, truncated) = preview_text(input.as_bytes());
        assert!(truncated);
        assert_eq!(text, input.chars().take(16000).collect::<String>());
        assert_eq!(
            preview_text("日本語 🚀".as_bytes()),
            ("日本語 🚀".into(), false)
        );
        assert_eq!(preview_text(&[b'a', 0xff]), ("a\u{fffd}".into(), false));
    }

    /// Newest first by default and by name on request; the filter hides rows but keeps
    /// their page index; Verify is offered for annotated tags only; Publish opens the
    /// remote tab, reads it and keeps the tag selected there; the × leaves its review.
    #[test]
    fn tags_page_orders_filters_and_offers_actions_by_kind() {
        common::isolated(
            "ROCKET_TAG_UI_CASE",
            "tags::tests::tags_page_orders_filters_and_offers_actions_by_kind",
            || {
                use crate::batch::flow_tests::{click, enabled, frame, open, wait};
                let f = common::Fixture::new();
                f.write("a.txt", "1\n");
                f.commit("First subject 文");
                f.run(&["tag", "light"]);
                for (date, name) in [("2026-03-01T00:00:00+00:00", "beta"), ("2026-02-01T00:00:00+00:00", "alpha")] {
                    let output = f
                        .command(&f.repository, &["tag", "--no-sign", "-a", "-m", "Notes", name])
                        .env("GIT_COMMITTER_DATE", date)
                        .output()
                        .unwrap();
                    assert!(output.status.success());
                }
                let origin = f.root.join("origin.git");
                f.run(&["init", "--bare", origin.to_str().unwrap()]);
                f.run(&["remote", "add", "origin", origin.to_str().unwrap()]);
                let ctx = Context::default();
                let mut app = open(&ctx, &f);
                app.dispatch(Intent::Dialog(DialogKind::Tags));
                wait(&ctx, &mut app);
                let tags = |app: &Rocket| -> (Vec<String>, Vec<TagSummary>) {
                    let page = app.dialog.as_ref().unwrap().tags.as_ref().unwrap().page.clone().unwrap();
                    let names = page.entries.iter().map(|e| display(&e.name)).collect();
                    (names, page.summaries)
                };
                let (names, summaries) = tags(&app);
                assert_eq!(names, ["beta", "alpha", "light"]);
                assert!(summaries.iter().all(|m| m.subject == "First subject 文".as_bytes()));
                // The filter hides rows; shown rows keep their page index.
                app.dialog.as_mut().unwrap().tags.as_mut().unwrap().filter = "LIG".into();
                frame(&ctx, &mut app, vec![]);
                assert!(app.frame_widgets.contains_key("dialog-tag-row-2"));
                assert!(!app.frame_widgets.contains_key("dialog-tag-row-0"));
                click(&ctx, &mut app, "dialog-tag-row-2");
                wait(&ctx, &mut app);
                frame(&ctx, &mut app, vec![]);
                assert!(!app.frame_widgets.contains_key("dialog-tag-verify"), "lightweight");
                assert!(enabled(&ctx, &mut app, "dialog-tag-publish"));
                app.dialog.as_mut().unwrap().tags.as_mut().unwrap().filter.clear();
                click(&ctx, &mut app, "dialog-tags-sort-name");
                wait(&ctx, &mut app);
                assert_eq!(tags(&app).0, ["alpha", "beta", "light"]);
                click(&ctx, &mut app, "dialog-tag-row-0");
                wait(&ctx, &mut app);
                assert!(enabled(&ctx, &mut app, "dialog-tag-verify"));
                click(&ctx, &mut app, "dialog-tag-publish");
                wait(&ctx, &mut app);
                let remote = &app.dialog.as_ref().unwrap().tags.as_ref().unwrap().remote;
                assert!(remote.active && remote.snapshot.is_some() && !remote.stale);
                assert_eq!(remote.selected.as_deref(), Some(&b"alpha"[..]));
                assert!(enabled(&ctx, &mut app, "dialog-remote-tag-publish"));
                assert!(!app.frame_widgets.contains_key("dialog-remote-tag-delete"), "only here: nothing to delete");
                // Its review has no Cancel: the header's ×, which keeps the old id,
                // returns to the remote tab.
                click(&ctx, &mut app, "dialog-remote-tag-publish");
                wait(&ctx, &mut app);
                assert!(app.dialog.as_ref().unwrap().tags.as_ref().unwrap().remote.action.is_some());
                assert!(enabled(&ctx, &mut app, "dialog-remote-tag-back"));
                assert_eq!(app.frame_widgets["dialog-remote-tag-back"].rect, app.frame_widgets["dialog-close"].rect);
                click(&ctx, &mut app, "dialog-close");
                let remote = &app.dialog.as_ref().unwrap().tags.as_ref().unwrap().remote;
                assert!(remote.active && remote.action.is_none(), "the × returns to the remote tab");
            },
        );
    }

    #[test]
    fn a_failure_splits_into_its_code_and_what_to_do() {
        assert_eq!(
            remote::failure_parts("Tag [remote_tag_limit]: Use Git to inspect it."),
            (Some("remote_tag_limit"), "Use Git to inspect it.")
        );
        assert_eq!(remote::failure_parts("Plain text [see below]"), (None, "Plain text [see below]"));
        assert_eq!(remote::failure_parts("Odd [Not A Code]: text"), (None, "Odd [Not A Code]: text"));
    }

    /// The read survives a look at the other tab; a tag changed here marks it stale; a
    /// verified publish flips its row with no further read.
    #[test]
    fn the_remote_read_stays_across_tabs_and_a_publish_updates_its_row() {
        common::isolated(
            "ROCKET_TAG_UI_CASE",
            "tags::tests::the_remote_read_stays_across_tabs_and_a_publish_updates_its_row",
            || {
                use crate::batch::flow_tests::{click, enabled, frame, open, wait};
                let f = common::Fixture::new();
                f.write("a.txt", "1\n");
                f.commit("First subject");
                f.run(&["tag", "both"]);
                f.run(&["tag", "here"]);
                let origin = f.root.join("origin.git");
                f.run(&["init", "--bare", origin.to_str().unwrap()]);
                f.run(&["remote", "add", "origin", origin.to_str().unwrap()]);
                f.run(&["push", "origin", "refs/tags/both:refs/tags/both"]);
                let ctx = Context::default();
                let mut app = open(&ctx, &f);
                app.dispatch(Intent::Dialog(DialogKind::Tags));
                wait(&ctx, &mut app);
                click(&ctx, &mut app, "dialog-tags-remote");
                click(&ctx, &mut app, "dialog-remote-tag-refresh");
                wait(&ctx, &mut app);
                frame(&ctx, &mut app, vec![]);
                fn remote(app: &Rocket) -> &RemoteState {
                    &app.dialog.as_ref().unwrap().tags.as_ref().unwrap().remote
                }
                assert_eq!(remote(&app).selected.as_deref(), Some(&b"both"[..]), "the first tag is selected");
                assert!(remote(&app).notice.is_none(), "a good read says nothing");

                click(&ctx, &mut app, "dialog-tags-local");
                wait(&ctx, &mut app);
                click(&ctx, &mut app, "dialog-tags-remote");
                frame(&ctx, &mut app, vec![]);
                assert!(remote(&app).active && remote(&app).snapshot.is_some(), "the read stays");
                assert!(!remote(&app).stale);

                // A tag made outside Rocket shows up in the local list, not in the read.
                click(&ctx, &mut app, "dialog-tags-local");
                wait(&ctx, &mut app);
                f.run(&["tag", "later"]);
                click(&ctx, &mut app, "dialog-preview-refresh");
                wait(&ctx, &mut app);
                click(&ctx, &mut app, "dialog-tags-remote");
                frame(&ctx, &mut app, vec![]);
                assert!(remote(&app).stale, "the read no longer matches");
                assert!(remote(&app).notice.as_deref().is_some_and(|n| n.contains("changed here")));
                assert!(!enabled(&ctx, &mut app, "dialog-remote-tags-batch"));
                click(&ctx, &mut app, "dialog-remote-tag-retry");
                wait(&ctx, &mut app);
                frame(&ctx, &mut app, vec![]);
                assert!(!remote(&app).stale);

                click(&ctx, &mut app, "dialog-remote-tags-here");
                frame(&ctx, &mut app, vec![]);
                assert_eq!(remote(&app).selected.as_deref(), Some(&b"here"[..]), "a hidden tag loses the selection");
                click(&ctx, &mut app, "dialog-remote-tag-publish");
                wait(&ctx, &mut app);
                click(&ctx, &mut app, "dialog-submit");
                wait(&ctx, &mut app);
                frame(&ctx, &mut app, vec![]);
                let r = remote(&app);
                assert!(!r.stale, "a verified publish needs no read");
                assert_eq!(r.done.as_deref(), Some("Published here to origin and verified."));
                let row = r.snapshot.as_ref().unwrap().entries.iter().find(|e| e.name == b"here").unwrap();
                assert_eq!(row.relation(), "published");

                // Publish on the other tab lands on its tag, whatever the last list showed.
                f.run(&["tag", "fresh"]);
                app.dialog.as_mut().unwrap().tags.as_mut().unwrap().remote.filter = "nothing".into();
                click(&ctx, &mut app, "dialog-tags-local");
                wait(&ctx, &mut app);
                click(&ctx, &mut app, "dialog-preview-refresh");
                wait(&ctx, &mut app);
                let tags = app.dialog.as_ref().unwrap().tags.as_ref().unwrap();
                let at = tags.page.as_ref().unwrap().entries.iter().position(|e| e.name == b"fresh").unwrap();
                click(&ctx, &mut app, &format!("dialog-tag-row-{at}"));
                wait(&ctx, &mut app);
                click(&ctx, &mut app, "dialog-tag-publish");
                frame(&ctx, &mut app, vec![]);
                assert_eq!(remote(&app).selected.as_deref(), Some(&b"fresh"[..]), "the read isn't back yet");
                wait(&ctx, &mut app);
                frame(&ctx, &mut app, vec![]);
                let r = remote(&app);
                assert_eq!(r.selected.as_deref(), Some(&b"fresh"[..]));
                assert!(r.filter.is_empty() && matches!(r.show, Show::All), "the tag is in the list");
            },
        );
    }

    #[test]
    fn switching_repository_dismisses_tag_review_without_writing() {
        common::isolated(
            "ROCKET_TAG_UI_CASE",
            "tags::tests::switching_repository_dismisses_tag_review_without_writing",
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
                assert_eq!(app.tabs.len(), 2);
                app.auto_refresh = false;
                app.active = 0;
                app.dispatch(Intent::Dialog(DialogKind::Tags));
                wait(&mut app);
                let dialog = app.dialog.as_mut().unwrap();
                dialog.first = "preview-only".into();
                let state = dialog.tags.as_mut().unwrap();
                state.action = Some(TagAction::Create);
                state.annotated = false;
                app.start_tag_read();
                wait(&mut app);
                assert!(
                    app.dialog
                        .as_ref()
                        .unwrap()
                        .tags
                        .as_ref()
                        .unwrap()
                        .preview
                        .is_some()
                );
                app.active = 1;
                let mut output = ctx.run_ui(Default::default(), |_| app.tag_dialog(&ctx));
                output.textures_delta.clear();
                assert!(app.dialog.is_none());
                assert!(app.write.is_none());
                for fixture in [&first, &second] {
                    assert!(
                        fixture
                            .repo()
                            .tags(200, &CancellationToken::new())
                            .unwrap()
                            .entries
                            .is_empty()
                    );
                }
            },
        );
    }
}
