//! Remote tag tab and pinned reviews. All Git work uses the backend workers.
use super::*;
use std::time::Instant;

/// Said when a list stays on screen but writes wait for a new read.
pub(super) const OUT_OF_DATE: &str = crate::i18n::mark("This list may be out of date.");

pub struct RemoteState {
    pub active: bool,
    pub snapshot: Option<Arc<RemoteTagSnapshot>>,
    pub selected: Option<Vec<u8>>,
    pub action: Option<RemoteTagAction>,
    pub preview: Option<Arc<RemoteTagPreview>>,
    pub list_task: Option<ReadTask<RemoteTagSnapshot>>,
    pub task: Option<ReadTask<RemoteTagPreview>>,
    pub stale: bool,
    pub notice: Option<String>,
    /// A change on the remote that was verified, until it is dismissed or read over.
    pub done: Option<String>,
    pub confirmation: String,
    pub filter: String,
    /// Which tags the list shows.
    pub show: Show,
    /// "Publish several": the tags chosen and their review.
    pub batch: Option<crate::batch::TagBatch<RemoteTagPreview>>,
    /// When the remote's tags were last read.
    pub read_at: Option<Instant>,
    /// A narrow window shows Details instead of Tags.
    pub details_tab: bool,
}
impl Default for RemoteState {
    fn default() -> Self {
        Self {
            active: false,
            snapshot: None,
            selected: None,
            action: None,
            preview: None,
            list_task: None,
            task: None,
            stale: true,
            notice: None,
            done: None,
            confirmation: String::new(),
            filter: String::new(),
            show: Show::All,
            batch: None,
            read_at: None,
            details_tab: false,
        }
    }
}
impl RemoteState {
    pub fn running(&self) -> bool {
        self.list_task.is_some()
            || self.task.is_some()
            || self
                .batch
                .as_ref()
                .is_some_and(crate::batch::TagBatch::running)
    }
    pub fn selected_entry(&self) -> Option<&RemoteTagEntry> {
        self.snapshot
            .as_ref()?
            .entries
            .iter()
            .find(|e| Some(&e.name) == self.selected.as_ref())
    }
    fn cancel(&mut self) {
        self.list_task = None;
        self.task = None;
        self.preview = None;
        self.stale = true;
        self.notice = self.snapshot.is_some().then(|| crate::i18n::tr("Read stopped. This list may be out of date.").into());
    }
    /// Shows a verified change without another read: the remote now holds what the
    /// review said it would.
    pub(crate) fn applied(&mut self, preview: &RemoteTagPreview) {
        let Some(snapshot) = self.snapshot.as_mut() else {
            return;
        };
        let snapshot = Arc::make_mut(snapshot);
        let name = &preview.entry().name;
        let Some(at) = snapshot.entries.iter().position(|e| e.name == *name) else {
            return;
        };
        let entry = &mut snapshot.entries[at];
        entry.remote_commit = None;
        match preview.expected_result() {
            Some(id) => {
                let annotated = entry.local.as_ref().is_some_and(TagEntry::annotated);
                entry.remote_id = Some(id.clone());
                entry.peeled = entry.summary.as_ref().filter(|_| annotated).and_then(|m| m.target.clone());
            }
            None if entry.local.is_some() => {
                entry.remote_id = None;
                entry.peeled = None;
            }
            None => {
                snapshot.entries.remove(at);
            }
        }
    }
}

/// Which tags the list shows, by where they stand.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Show {
    #[default]
    All,
    Here,
    There,
    Different,
    Same,
    Indirect,
}
impl Show {
    const EVERY: [Self; 6] = [Self::All, Self::Here, Self::There, Self::Different, Self::Same, Self::Indirect];
    fn label(self, there: &str) -> String {
        match self {
            Self::All => crate::i18n::tr("All").into(),
            Self::Here => crate::i18n::tr("Only here").into(),
            Self::There => there.into(),
            Self::Different => crate::i18n::tr("Different").into(),
            Self::Same => crate::i18n::tr("Same").into(),
            Self::Indirect => crate::i18n::tr("Indirect").into(),
        }
    }
    fn id(self) -> &'static str {
        match self {
            Self::All => "dialog-remote-tags-all",
            Self::Here => "dialog-remote-tags-here",
            Self::There => "dialog-remote-tags-there",
            Self::Different => "dialog-remote-tags-different",
            Self::Same => "dialog-remote-tags-same",
            Self::Indirect => "dialog-remote-tags-indirect",
        }
    }
    fn shows(self, relation: &str) -> bool {
        match self {
            Self::All => true,
            Self::Here => relation == "local_only",
            Self::There => relation == "remote_only",
            Self::Different => relation == "different",
            Self::Same => relation == "published",
            Self::Indirect => relation == "symbolic",
        }
    }
}

/// Where a tag stands between here and the remote, as its row says it.
pub(crate) fn relation(entry: &RemoteTagEntry, remote: &str) -> (String, Color32) {
    match entry.relation() {
        "local_only" => (crate::i18n::tr("Only here").into(), BLUE()),
        "published" => (crate::i18n::tr("Same on both").into(), GREEN()),
        "different" => (crate::i18n::trf!("Different on {remote}", remote), AMBER()),
        "remote_only" => (crate::i18n::trf!("Only on {remote}", remote), MUTED()),
        _ => (crate::i18n::tr("Indirect").into(), HINT()),
    }
}

/// The mark a row shows for where its tag stands.
fn relation_mark(entry: &RemoteTagEntry) -> (Icon, Color32) {
    match entry.relation() {
        "local_only" => (Icon::ArrowUp, BLUE()),
        "published" => (Icon::Equal, GREEN()),
        "different" => (Icon::NotEqual, AMBER()),
        "remote_only" => (Icon::ArrowDown, MUTED()),
        _ => (Icon::Link, HINT()),
    }
}

/// A remote list row's kind, commit and subject: the local tag's when there is one.
fn remote_row_text(
    entry: &RemoteTagEntry,
    remote: &str,
) -> (Option<(&'static str, Color32)>, String, String) {
    let there = entry.peeled.as_ref().or(entry.remote_id.as_ref());
    if let Some(target) = &entry.symbolic_target {
        let target = display(target);
        let branch = target.strip_prefix("refs/heads/").unwrap_or(&target);
        return (None, there.map_or("", short).to_owned(), crate::i18n::trf!("Follows {branch} on {remote}", branch, remote));
    }
    match &entry.local {
        Some(local) => {
            let summary = entry.summary.as_ref();
            let sha = summary
                .and_then(|m| m.target.as_ref())
                .map_or(short(&local.id), short)
                .to_owned();
            (Some(kind(local)), sha, summary.map_or_else(String::new, |m| display(&m.subject)))
        }
        None => {
            // Only a tag object is advertised with a peeled target.
            let kind = if entry.peeled.is_some() {
                (crate::i18n::tr("Annotated"), pages::INFO())
            } else {
                (crate::i18n::tr("Lightweight"), pages::NEUTRAL())
            };
            let subject = entry
                .remote_commit
                .as_ref()
                .map_or_else(|| crate::i18n::tr("Commit not fetched").to_owned(), |c| display(&c.subject));
            (Some(kind), there.map_or("", short).to_owned(), subject)
        }
    }
}

/// The rows the list shows, by index into the snapshot.
fn remote_rows(r: &RemoteState, snapshot: &RemoteTagSnapshot) -> Vec<usize> {
    let needle = r.filter.trim().to_lowercase();
    snapshot
        .entries
        .iter()
        .enumerate()
        .filter(|(_, e)| {
            r.show.shows(e.relation())
                && (needle.is_empty() || display(&e.name).to_lowercase().contains(&needle))
        })
        .map(|(i, _)| i)
        .collect()
}

/// A failure's fixed text: its code, if it has one, and what to do.
pub(super) fn failure_parts(text: &str) -> (Option<&str>, &str) {
    let parts = text.split_once("]: ").and_then(|(head, rest)| {
        let code = head.rsplit('[').next()?;
        code.bytes()
            .all(|b| b.is_ascii_lowercase() || b == b'_')
            .then_some((code, rest))
    });
    match parts {
        Some((code, rest)) => (Some(code), rest),
        None => (None, text),
    }
}

/// How long ago the remote was last read.
fn read_when(at: Instant) -> String {
    let seconds = at.elapsed().as_secs();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    ago(seconds, now - seconds as i64)
}

/// What a click on the details card asked for.
enum Ask {
    Review(RemoteTagAction),
    Show(ObjectId),
}

impl Rocket {
    pub(super) fn remote_tags_active(&self) -> bool {
        self.dialog
            .as_ref()
            .and_then(|d| d.tags.as_ref())
            .is_some_and(|s| s.remote.active)
    }
    pub(super) fn start_remote_tag_read(&mut self) {
        let Some(d) = &mut self.dialog else {
            return;
        };
        let Some(s) = &mut d.tags else {
            return;
        };
        if s.applying {
            return;
        }
        let r = &mut s.remote;
        r.list_task = None;
        r.task = None;
        r.preview = None;
        d.error = None;
        r.notice = None;
        r.done = None;
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
        if let Some(action) = r.action {
            let (Some(snapshot), Some(name)) = (r.snapshot.clone(), r.selected.clone()) else {
                return;
            };
            match ReadTask::spawn(move |cancel| {
                backend.remote_tag_preview(&path, &snapshot, &name, action, &cancel)
            }) {
                Ok(task) => r.task = Some(task),
                Err(e) => d.error = Some(user_error(&e)),
            }
        } else {
            r.stale = true;
            let Some(remote) = d.remotes.get(d.remote).cloned() else {
                d.error = Some(crate::i18n::tr("Add a remote in Repository → Remotes & Hosting → Manage Remotes first.").into());
                return;
            };
            match ReadTask::spawn(move |cancel| backend.remote_tags(&path, &remote, &cancel)) {
                Ok(task) => r.list_task = Some(task),
                Err(e) => d.error = Some(user_error(&e)),
            }
        }
    }
    pub(super) fn poll_remote_tag_read(&mut self) {
        let Some(d) = &mut self.dialog else {
            return;
        };
        let Some(s) = &mut d.tags else {
            return;
        };
        let r = &mut s.remote;
        if let Some(result) = r.list_task.as_mut().and_then(ReadTask::try_finish) {
            r.list_task = None;
            match result {
                Ok(snapshot) => {
                    r.selected = r
                        .selected
                        .take()
                        .filter(|name| snapshot.entries.iter().any(|e| e.name == *name));
                    r.snapshot = Some(Arc::new(snapshot));
                    r.stale = false;
                    r.read_at = Some(Instant::now());
                }
                Err(e) => {
                    r.stale = true;
                    d.error = Some(user_error(&e));
                }
            }
        }
        if let Some(result) = r.task.as_mut().and_then(ReadTask::try_finish) {
            r.task = None;
            match result {
                Ok(preview) => r.preview = Some(Arc::new(preview)),
                Err(e) => {
                    r.stale = true;
                    d.error = Some(user_error(&e));
                }
            }
        }
    }
    pub(super) fn finish_remote_tag_dialog(&mut self, owner: Option<u64>, outcome: &Outcome) {
        let Some(d) = &mut self.dialog else {
            return;
        };
        let remote = remote_name(d);
        let Some(s) = &mut d.tags else {
            return;
        };
        if !s.applying || d.owner != owner {
            return;
        }
        s.applying = false;
        let r = &mut s.remote;
        let preview = r.preview.take();
        r.action = None;
        r.confirmation.clear();
        // No automatic retry or additional network refresh after a write. The
        // backend's bounded observation already reports its actual outcome.
        if let Some(p) = preview.filter(|_| outcome.command_succeeded && !outcome.recovery_pending) {
            r.applied(&p);
            let name = display(&p.entry().name);
            r.done = Some(match p.action() {
                RemoteTagAction::Publish => crate::i18n::trf!("Published {name} to {remote} and verified.", name, remote),
                RemoteTagAction::Delete => crate::i18n::trf!("Deleted {name} from {remote} and verified.", name, remote),
            });
            return;
        }
        r.stale = true;
        r.notice = Some(crate::i18n::trf!(
            "{summary}\n{details}\nRead again before another change.",
            summary = outcome.summary, details = outcome.details.join("\n")
        ));
        if !outcome.command_succeeded {
            d.error = Some(outcome.summary.clone());
        }
    }

    /// Tags page, remote tab: the selected remote's tags beside this repository's.
    pub(super) fn remote_tag_page(&mut self, ui: &mut Ui) {
        let writable = self.can_write_repository();
        let default_target = self.default_tag_target();
        let Some(mut d) = self.dialog.take() else {
            return;
        };
        let remote = remote_name(&d);
        let has_remote = !d.remotes.is_empty();
        let s = d.tags.get_or_insert_with(Default::default);
        let reviewing = s.reviewing();
        let waiting = s.remote.running() || s.applying;
        let mut refresh = false;
        let mut close = false;
        let mut local = false;
        let mut create = false;
        let mut show_commit = None;
        let mut inert = HashMap::new();
        let widgets = if reviewing {
            &mut inert
        } else {
            &mut self.frame_widgets
        };
        ui.add_enabled_ui(!reviewing, |ui| {
            let clicks = top(
                ui,
                widgets,
                Top {
                    remote: true,
                    count: s.page.as_ref().map(|p| p.entries.len()),
                    remote_count: s.remote.snapshot.as_ref().map(|p| p.entries.len()),
                    remote_name: &remote,
                    can_close: !s.applying,
                    can_create: !waiting && writable,
                    can_refresh: None,
                    can_switch: !waiting,
                },
            );
            close = clicks.close;
            local = clicks.local;
            create = clicks.new_tag;
            let tabs_bottom = ui.cursor().top();
            let tabs_range = ui.max_rect().x_range();
            let r = &mut s.remote;
            let narrow = ui.available_width() - 48.0 < 700.0;
            // Below this the count buttons fill the row, so the read moves up beside the tabs.
            let read_above = ui.available_width() - 48.0 < 980.0;
            let several = d.remotes.len() > 1;
            let there = crate::i18n::trf!("Only on {remote}", remote);
            if several || r.snapshot.is_some() {
                pages::toolbar(ui, |ui| {
                    if several {
                        let choice = ComboBox::from_id_salt("remote-tag-destination")
                            .selected_text(
                                d.remotes
                                    .get(d.remote)
                                    .map(|b| display(b))
                                    .unwrap_or_else(|| crate::i18n::tr("No remote").into()),
                            )
                            .width(120.0)
                            .icon(|ui, rect, _, _| icon(ui.painter(), rect.shrink(2.0), Icon::Chevron, HINT()))
                            .show_ui(ui, |ui| {
                                for (index, name) in d.remotes.iter().enumerate() {
                                    let b = ui.add_enabled(
                                        !waiting,
                                        Button::selectable(index == d.remote, display(name)),
                                    );
                                    control(widgets, format!("dialog-remote-tag-remote-{index}"), &b, ui.clip_rect());
                                    if b.clicked() && index != d.remote {
                                        d.remote = index;
                                        r.snapshot = None;
                                        r.selected = None;
                                        r.stale = true;
                                        r.notice = None;
                                        r.done = None;
                                        r.read_at = None;
                                        d.error = None;
                                    }
                                }
                            });
                        control(widgets, "dialog-remote-tag-remote", &choice.response, ui.clip_rect());
                    }
                    let Some(snapshot) = r.snapshot.clone() else {
                        return;
                    };
                    let width = if narrow { 150.0 } else { 240.0 };
                    let b = pages::search_box(ui, &mut r.filter, "dialog-remote-tag-filter", crate::i18n::tr("Filter tags"), width);
                    control(widgets, "dialog-remote-tag-filter", &b, ui.clip_rect());
                    let count = |show: Show| snapshot.entries.iter().filter(|e| show.shows(e.relation())).count();
                    // Indirect is rare; its button only shows when there is one.
                    let shows: Vec<Show> = Show::EVERY
                        .into_iter()
                        .filter(|show| *show != Show::Indirect || count(*show) > 0)
                        .collect();
                    let labels: Vec<String> = shows.iter().map(|show| show.label(&there)).collect();
                    let items: Vec<(&str, usize, bool)> = shows
                        .iter()
                        .zip(&labels)
                        .map(|(show, label)| (label.as_str(), count(*show), r.show == *show))
                        .collect();
                    for (b, show) in pages::count_segments(ui, &items).iter().zip(&shows) {
                        control(widgets, show.id(), b, ui.clip_rect());
                        if b.clicked() {
                            r.show = *show;
                        }
                    }
                    if !read_above {
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            refresh |= read_again(ui, widgets, r, !waiting && has_remote, 100.0);
                        });
                    }
                });
            }
            if read_above && r.snapshot.is_some() {
                let row = Rect::from_x_y_ranges(tabs_range, (tabs_bottom - 38.0)..=tabs_bottom).shrink2(vec2(24.0, 4.0));
                let mut slot = ui.new_child(UiBuilder::new().max_rect(row).layout(Layout::right_to_left(Align::Center)));
                refresh |= read_again(&mut slot, widgets, r, !waiting && has_remote, if narrow { 30.0 } else { 100.0 });
            }
            let read_failed = d.error.is_some() && r.notice.is_none() && r.snapshot.is_some();
            let feedback = r.notice.is_some() || r.done.is_some() || (d.error.is_some() && r.snapshot.is_some());
            if feedback {
                // A column, so the notice is only as tall as its text.
                let margin = Margin { left: 24, right: 24, top: 0, bottom: 8 };
                Frame::new().inner_margin(margin).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    let feedback = ScrollArea::vertical()
                        .id_salt("remote-tag-feedback")
                        .max_height((ui.clip_rect().height() * 0.25).clamp(96.0, 140.0))
                        .auto_shrink([false, true])
                        .show(ui, |ui| {
                            ui.spacing_mut().item_spacing.y = 6.0;
                            if read_failed {
                                let when = r.read_at.map_or_else(|| crate::i18n::tr("an earlier read").to_owned(), read_when);
                                let (_, why) = failure_parts(d.error.as_deref().unwrap_or_default());
                                let text = crate::i18n::trf!("Couldn't read {remote}. This list is from {when}, so changes wait for a new read.\n{why}", remote, when, why);
                                if let Some(b) = pages::notice_strip(ui, RED(), &text, Some(crate::i18n::tr("Try again"))) {
                                    control(widgets, "dialog-remote-tag-retry", &b, ui.clip_rect());
                                    refresh |= b.clicked();
                                }
                            } else if let (Some(error), Some(notice)) = (&d.error, &r.notice)
                                && notice.starts_with(error)
                            {
                                pages::notice_strip(ui, RED(), notice, None);
                            } else {
                                if let Some(error) = &d.error {
                                    pages::notice_strip(ui, RED(), error, None);
                                }
                                if let Some(notice) = &r.notice {
                                    let action = r.stale.then_some(crate::i18n::tr("Read again"));
                                    if let Some(b) = pages::notice_strip(ui, AMBER(), notice, action) {
                                        control(widgets, "dialog-remote-tag-retry", &b, ui.clip_rect());
                                        refresh |= b.clicked();
                                    }
                                }
                            }
                            if let Some(done) = &r.done
                                && pages::done_strip(ui, GREEN(), done, widgets, "dialog-remote-tag-dismiss")
                            {
                                r.done = None;
                            }
                        });
                    pages::region(
                        widgets,
                        "dialog-remote-tag-feedback",
                        feedback.inner_rect.intersect(ui.clip_rect()),
                        !s.applying,
                    );
                });
            }
            let region = ui.available_rect_before_wrap();
            if r.snapshot.is_some() {
                let can_publish = !waiting && !r.stale && writable && d.error.is_none();
                let ready = can_publish;
                keep_selection(r);
                let asked = if narrow {
                    remote_narrow(ui, r, waiting, widgets, &remote, ready, can_publish)
                } else {
                    let mut asked = None;
                    pages::split(ui, |ui, side| match side {
                        pages::Side::List => remote_list(ui, r, waiting, widgets, &remote, Some(can_publish), false),
                        pages::Side::Details => asked = remote_details(ui, r, widgets, &remote, ready, false),
                    });
                    pages::details_scroll(ui, widgets, !s.applying);
                    asked
                };
                match asked {
                    Some(Ask::Review(action)) => {
                        r.action = Some(action);
                        r.confirmation.clear();
                        refresh = true;
                    }
                    Some(Ask::Show(id)) => show_commit = Some(id),
                    None => {}
                }
            } else if r.running() {
                pages::loading_with(ui, &crate::i18n::trf!("Reading tags on {remote}", remote), 40.0, |ui| {
                    ui.add_space(8.0);
                    let b = ui.add(Button::new(crate::i18n::tr("Stop")));
                    control(widgets, "dialog-remote-tag-cancel", &b, ui.clip_rect());
                    if b.clicked() {
                        r.cancel();
                    }
                });
            } else {
                refresh |= unread(ui, widgets, &remote, has_remote, d.error.as_deref(), !waiting);
            }
            pages::region(widgets, "dialog-tags-scroll", region.intersect(ui.clip_rect()), !s.applying);
        });
        if local || create {
            // The read stays for the way back.
            s.remote.active = false;
            s.remote.done = None;
            d.error = None;
            if create {
                // The list is shown again when the review closes.
                s.action = Some(TagAction::Create);
                s.target = default_target;
                s.invalidate();
            } else {
                s.read_list = true;
                refresh = true;
            }
        }
        if !close {
            self.dialog = Some(d);
        }
        if refresh {
            self.start_tag_read();
        }
        if let Some(id) = show_commit {
            self.close_browsing_page();
            self.dispatch(Intent::SelectCommit(id));
        }
    }

    pub(super) fn remote_tag_dialog(&mut self, ctx: &Context) {
        let writable = self.can_write_repository();
        let Some(mut d) = self.dialog.take() else {
            return;
        };
        let remote = remote_name(&d);
        let s = d.tags.get_or_insert_with(Default::default);
        if d.owner != self.tab().map(|t| t.id) && !s.applying {
            return;
        }
        let r = &mut s.remote;
        let Some(action) = r.action else {
            self.dialog = Some(d);
            return;
        };
        let delete = action == RemoteTagAction::Delete;
        let mut submit = false;
        let mut cancel = false;
        let waiting = r.running() || s.applying;
        let name = r.selected.as_deref().map(display).unwrap_or_default();
        let title = if delete {
            crate::i18n::trf!("Delete {name} from {remote}?", name, remote)
        } else {
            crate::i18n::trf!("Publish {name}", name)
        };
        let subtitle = if delete {
            crate::i18n::trf!("Removes the tag from {remote}. There is no undo on the remote.", remote)
        } else {
            crate::i18n::trf!("Sends this tag to {remote} so others can fetch it.", remote)
        };
        let head = pages::ReviewHeader {
            image: Icon::Tag,
            title: &title,
            subtitle: &subtitle,
            danger: delete,
        };
        let running = pages::Running::of(self.write.as_ref());
        let response = pages::review(
            ctx,
            "rocket-remote-tag-review",
            head,
            &mut self.frame_widgets,
            "dialog-tags-scroll",
            !s.applying,
            |ui, part, widgets| match part {
                pages::Part::Body => {
                    if let Some(p) = &r.preview {
                        let entry = p.entry();
                        pages::titled_card(
                            ui,
                            |ui| {
                                let title = if delete { crate::i18n::trf!("Deletes {name}", name) } else { crate::i18n::trf!("Publishes {name}", name) };
                                ui.label(RichText::new(title).strong());
                            },
                            |ui| {
                                pages::key_value(ui, if delete { crate::i18n::tr("From") } else { crate::i18n::tr("To") }, 112.0, |ui| {
                                    let mut job = text::LayoutJob::default();
                                    job.append(&display(p.remote()), 0.0, TextFormat::simple(FontId::proportional(13.0), TEXT()));
                                    job.append(&format!(" · {}", p.destination_display()), 0.0, TextFormat::simple(FontId::proportional(13.0), FAINT()));
                                    job.wrap = text::TextWrapping::truncate_at_width(ui.available_width());
                                    ui.label(job).on_hover_text(p.destination_display());
                                });
                                pages::key_value(ui, crate::i18n::tr("Points to"), 112.0, |ui| {
                                    let (_, sha, subject) = remote_row_text(entry, &remote);
                                    let mut job = text::LayoutJob::default();
                                    job.append(&sha, 0.0, TextFormat::simple(FontId::monospace(12.5), TEXT()));
                                    if !subject.is_empty() {
                                        job.append(&format!(" · {subject}"), 0.0, TextFormat::simple(FontId::proportional(13.0), TEXT()));
                                    }
                                    job.wrap = text::TextWrapping::truncate_at_width(ui.available_width());
                                    ui.label(job);
                                });
                                pages::key_value(ui, &crate::i18n::trf!("On {remote} now", remote), 112.0, |ui| {
                                    match (&entry.remote_id, delete) {
                                        (None, _) => check(ui, true, crate::i18n::tr("Not there yet · nothing will be replaced")),
                                        (Some(id), true) => {
                                            ui.label(RichText::new(crate::i18n::trf!("Has {name} at {commit}", name, commit = short(id))).color(TEXT()));
                                        }
                                        (Some(id), false) => {
                                            ui.label(RichText::new(crate::i18n::trf!("Has {name} at {commit}", name, commit = short(id))).color(AMBER()));
                                        }
                                    }
                                });
                                pages::key_value(ui, crate::i18n::tr("Result"), 112.0, |ui| {
                                    ui.label(match p.expected_result() {
                                        Some(id) => crate::i18n::trf!("{name} on {remote} at {commit}", name, remote, commit = short(id)),
                                        None => crate::i18n::trf!("No {name} on {remote}", name, remote),
                                    });
                                });
                            },
                        );
                        if delete {
                            pages::callout(
                                ui,
                                AMBER(),
                                crate::i18n::tr("Anyone who already fetched the tag keeps it. There is no remote undo or backup."),
                            );
                            ui.label(crate::i18n::tr("Type the exact tag name to confirm deletion"));
                            let b = ui.add_enabled(
                                !s.applying,
                                TextEdit::singleline(&mut r.confirmation)
                                    .desired_width(f32::INFINITY)
                                    .min_size(vec2(0.0, 30.0))
                                    .vertical_align(Align::Center),
                            );
                            control(widgets, "dialog-remote-tag-confirm", &b, ui.clip_rect());
                        }
                    }
                    // Updating the remote shows its status and Stop in the footer.
                    if waiting && !s.applying {
                        pages::waiting(ui, crate::i18n::tr("Rechecking the remote and the tag…"));
                    }
                    if let Some(error) = &d.error {
                        pages::callout(ui, RED(), error);
                    }
                    if let Some(notice) = &r.notice {
                        pages::callout(ui, AMBER(), notice);
                    }
                }
                pages::Part::Footer if s.applying => {
                    cancel = pages::running_footer(ui, widgets, &running, &[]);
                }
                pages::Part::Footer => {
                    let ready = !waiting
                        && !r.stale
                        && writable
                        && d.error.is_none()
                        && r.preview.as_ref().is_some_and(|p| {
                            !delete || r.confirmation.as_bytes() == p.entry().name
                        });
                    let b = if delete {
                        pages::danger(ui, ready, &crate::i18n::trf!("Delete from {remote}", remote))
                    } else {
                        pages::primary(ui, ready, crate::i18n::tr("Publish tag"))
                    };
                    control(widgets, "dialog-submit", &b, ui.clip_rect());
                    submit = b.clicked();
                    if r.running() {
                        let b = pages::ghost(ui, true, crate::i18n::tr("Stop"));
                        control(widgets, "dialog-remote-tag-cancel", &b, ui.clip_rect());
                        if b.clicked() {
                            r.cancel();
                        }
                    }
                    let status = if delete && r.preview.is_some() && !ready && !waiting && d.error.is_none() {
                        crate::i18n::tr("Reviewed · type the tag name to delete it")
                    } else {
                        review_status(waiting, d.error.is_some(), r.preview.is_some())
                    };
                    pages::footer_status(ui, false, status);
                }
            },
        );
        // The × and Esc return to the remote tab; a click outside does not.
        // Automation's old Cancel id is that ×.
        if let Some(close) = self.frame_widgets.get("dialog-close").copied() {
            self.frame_widgets.insert("dialog-remote-tag-back".into(), close);
        }
        let back = (response.close_clicked() || ctx.input(|i| i.key_pressed(Key::Escape))) && !s.applying;
        if back {
            r.action = None;
            r.task = None;
            r.preview = None;
            r.confirmation.clear();
            d.error = None;
            if r.stale && r.notice.is_none() {
                r.notice = Some(crate::i18n::tr(OUT_OF_DATE).into());
            }
        }
        if submit && let (Some(owner), Some(p)) = (d.owner, r.preview.clone()) {
            s.applying = true;
            self.execute(owner, Action::RemoteTag(p));
            if self.write.is_none() {
                s.applying = false;
                d.error = Some(crate::i18n::tr("Could not start; wait for the repository and review again.").into());
            }
        }
        self.dialog = Some(d);
        if cancel {
            self.cancel_write();
        }
    }
}

/// When the remote was read and Read again, or Stop while a read runs. One place for
/// both, so nothing beside them moves. Returns whether Read again was clicked.
fn read_again(
    ui: &mut Ui,
    widgets: &mut HashMap<String, UiControl>,
    r: &mut RemoteState,
    enabled: bool,
    width: f32,
) -> bool {
    let mut clicked = false;
    if r.running() {
        let b = pages::slot_button(ui, None, crate::i18n::tr("Stop"), width, true);
        control(widgets, "dialog-remote-tag-cancel", &b, ui.clip_rect());
        if b.clicked() {
            r.cancel();
        }
    } else {
        let b = pages::slot_button(ui, Some(Icon::Refresh), crate::i18n::tr("Read again"), width, enabled);
        control(widgets, "dialog-remote-tag-refresh", &b, ui.clip_rect());
        clicked = b.clicked();
    }
    if let Some(at) = r.read_at {
        ui.label(RichText::new(crate::i18n::trf!("Read {when}", when = read_when(at))).size(12.0).color(HINT()));
    }
    clicked
}

/// The tab with no read to show: the first read to make, or why it failed. Returns
/// whether Read or Try again was clicked.
fn unread(
    ui: &mut Ui,
    widgets: &mut HashMap<String, UiControl>,
    remote: &str,
    has_remote: bool,
    error: Option<&str>,
    enabled: bool,
) -> bool {
    ui.add_space(64.0);
    ui.vertical_centered(|ui| {
        ui.set_max_width(460.0);
        ui.spacing_mut().item_spacing.y = 8.0;
        if let Some(error) = error.filter(|_| has_remote) {
            let (code, why) = failure_parts(error);
            let (mark, _) = ui.allocate_exact_size(vec2(24.0, 24.0), Sense::hover());
            icon(ui.painter(), mark, Icon::Warning, AMBER());
            let limit = code == Some("remote_tag_limit");
            let title = if limit { crate::i18n::tr("Too many tags to compare").to_owned() } else { crate::i18n::trf!("Couldn't read tags on {remote}", remote) };
            ui.label(RichText::new(title).font(semibold(15.0)));
            ui.add(Label::new(RichText::new(why).color(HINT())).wrap());
            if let Some(code) = code {
                ui.label(RichText::new(code).monospace().size(11.5).color(MUTED()));
            }
            if limit {
                return false;
            }
            ui.add_space(4.0);
            let r = pages::primary(ui, enabled, crate::i18n::tr("Try again"));
            control(widgets, "dialog-remote-tag-refresh", &r, ui.clip_rect());
            return r.clicked();
        }
        let (badge, _) = ui.allocate_exact_size(vec2(44.0, 44.0), Sense::hover());
        ui.painter().circle_filled(badge.center(), 22.0, CARD());
        icon(ui.painter(), badge.shrink(12.0), Icon::Fetch, BLUE());
        if !has_remote {
            ui.label(RichText::new(crate::i18n::tr("This repository has no remotes")).font(semibold(15.0)));
            ui.label(RichText::new(crate::i18n::tr("Add one in Repository → Remotes & Hosting → Manage Remotes to compare tags.")).color(HINT()));
            return false;
        }
        ui.label(RichText::new(crate::i18n::trf!("Tags on {remote} haven't been read yet", remote)).font(semibold(15.0)));
        ui.label(
            RichText::new(crate::i18n::trf!("Rocket contacts {remote} only when you ask. Reading changes nothing.", remote))
                .color(HINT()),
        );
        ui.add_space(4.0);
        let r = pages::primary(ui, enabled, &crate::i18n::trf!("Read tags on {remote}", remote));
        control(widgets, "dialog-remote-tag-refresh", &r, ui.clip_rect());
        r.clicked()
    })
    .inner
}

/// A shown tag stays selected; otherwise the first row is, so details are never blank.
/// A read in progress may be about to bring the tag that was asked for.
fn keep_selection(r: &mut RemoteState) {
    let Some(snapshot) = r.snapshot.clone().filter(|_| r.list_task.is_none()) else {
        return;
    };
    let rows = remote_rows(r, &snapshot);
    if !rows.iter().any(|i| Some(&snapshot.entries[*i].name) == r.selected.as_ref()) {
        r.selected = rows.first().map(|i| snapshot.entries[*i].name.clone());
    }
}

/// The tags. `publish` is whether Publish several can start, or `None` when the caller
/// shows it elsewhere.
fn remote_list(
    ui: &mut Ui,
    r: &mut RemoteState,
    waiting: bool,
    widgets: &mut HashMap<String, UiControl>,
    remote: &str,
    publish: Option<bool>,
    narrow: bool,
) {
    let Some(snapshot) = r.snapshot.clone() else {
        return;
    };
    let rows = remote_rows(r, &snapshot);
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        ScrollArea::vertical()
            .id_salt("remote-tags-list")
            .max_height((ui.available_height() - if publish.is_some() { 40.0 } else { 0.0 }).max(60.0))
            .auto_shrink([false, true])
            .show_rows(ui, 50.0, rows.len(), |ui, range| {
                for &index in &rows[range] {
                    let entry = &snapshot.entries[index];
                    let name = display(&entry.name);
                    let (status, color) = relation(entry, remote);
                    let (kind, sha, subject) = remote_row_text(entry, remote);
                    let b = tag_row(
                        ui,
                        TagRow {
                            name: &name,
                            tile: Some(relation_mark(entry)),
                            kind,
                            sha: &sha,
                            subject: &subject,
                            meta: &status,
                            meta_color: color,
                        },
                        r.selected.as_ref() == Some(&entry.name),
                        !waiting,
                    );
                    control(widgets, format!("dialog-remote-tag-row-{index}"), &b, ui.clip_rect());
                    if b.clicked() {
                        r.selected = Some(entry.name.clone());
                        r.details_tab = narrow;
                    }
                }
            });
    });
    if rows.is_empty() {
        ui.add_space(36.0);
        ui.vertical_centered(|ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            if snapshot.entries.is_empty() {
                ui.label(RichText::new(crate::i18n::tr("No tags here or on the remote yet.")).color(HINT()));
                return;
            }
            ui.label(RichText::new(crate::i18n::tr("No tags match")).font(semibold(14.0)));
            let needle = r.filter.trim();
            if !needle.is_empty() {
                ui.label(RichText::new(crate::i18n::trf!("Nothing for “{needle}”.", needle)).color(HINT()));
            }
            ui.add_space(4.0);
            let b = ui.add(Button::new(crate::i18n::tr("Clear filter")));
            control(widgets, "dialog-remote-tag-clear", &b, ui.clip_rect());
            if b.clicked() {
                r.filter.clear();
                r.show = Show::All;
            }
        });
    }
    if let Some(publish) = publish {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let total = snapshot.entries.len();
            let mut text = if rows.len() == total {
                crate::i18n::trf!("Tags: {count}", count = pages::number_text(total))
            } else {
                crate::i18n::trf!("Tags: {shown} of {total}", shown = pages::number_text(rows.len()), total = pages::number_text(total))
            };
            let can = crate::batch::publish_items(r, "").len();
            if can > 0 {
                text.push_str(&crate::i18n::trf!(" · can be published: {count}", count = pages::number_text(can)));
            }
            ui.label(RichText::new(text).size(12.0).color(FAINT()));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                publish_several(ui, r, widgets, publish);
            });
        });
    }
}

fn publish_several(
    ui: &mut Ui,
    r: &mut RemoteState,
    widgets: &mut HashMap<String, UiControl>,
    enabled: bool,
) {
    let some = !crate::batch::publish_items(r, "").is_empty();
    let b = ui.add_enabled(enabled && some, Button::new(crate::i18n::tr("Publish several")));
    control(widgets, "dialog-remote-tags-batch", &b, ui.clip_rect());
    if b.clicked() {
        r.batch = Some(Default::default());
        r.notice = None;
        r.done = None;
    }
}

/// A narrow window: Tags and Details as tabs in one card.
fn remote_narrow(
    ui: &mut Ui,
    r: &mut RemoteState,
    waiting: bool,
    widgets: &mut HashMap<String, UiControl>,
    remote: &str,
    ready: bool,
    can_publish: bool,
) -> Option<Ask> {
    let area = ui.available_rect_before_wrap();
    let rect = Rect::from_min_max(area.min + vec2(24.0, 0.0), area.max - vec2(24.0, 12.0));
    let mut asked = None;
    pages::card_back(ui, rect);
    ui.scope_builder(UiBuilder::new().max_rect(rect), |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        pages::card_bar(ui, |ui| {
            let count = r.snapshot.as_ref().map(|p| p.entries.len());
            let items = [(crate::i18n::tr("Tags"), count, !r.details_tab), (crate::i18n::tr("Details"), None, r.details_tab)];
            for (b, (id, details)) in pages::tab_segments(ui, &items)
                .into_iter()
                .zip([("dialog-remote-tags-list", false), ("dialog-remote-tags-details", true)])
            {
                control(widgets, id, &b, ui.clip_rect());
                if b.clicked() {
                    r.details_tab = details;
                }
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if !r.details_tab {
                    publish_several(ui, r, widgets, can_publish);
                } else if let Some(entry) = r.selected_entry().cloned() {
                    asked = remote_actions(ui, &entry, widgets, remote, ready);
                }
            });
        });
        let rest = Rect::from_min_max(ui.cursor().min + vec2(6.0, 4.0), rect.max - vec2(6.0, 6.0));
        ui.scope_builder(UiBuilder::new().max_rect(rest), |ui| {
            ui.set_clip_rect(rest.intersect(ui.clip_rect()));
            if r.details_tab {
                let out = ScrollArea::vertical()
                    .id_salt("remote-tag-details")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.add_space(4.0);
                        Frame::new().inner_margin(Margin::symmetric(8, 0)).show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            if let Some(ask) = remote_details(ui, r, widgets, remote, ready, true) {
                                asked = Some(ask);
                            }
                        });
                    });
                pages::region(widgets, "page-details-scroll", out.inner_rect.intersect(ui.clip_rect()), true);
            } else {
                remote_list(ui, r, waiting, widgets, remote, None, true);
            }
        });
    });
    ui.allocate_rect(rect, Sense::hover());
    asked
}

/// One side of a tag: its commit, its subject and when.
struct Side {
    sha: String,
    subject: String,
    when: Option<String>,
}

fn here_side(entry: &RemoteTagEntry) -> Option<Side> {
    let local = entry.local.as_ref()?;
    let summary = entry.summary.as_ref();
    Some(Side {
        sha: summary.and_then(|m| m.target.as_ref()).map_or(short(&local.id), short).to_owned(),
        subject: summary.map_or_else(String::new, |m| display(&m.subject)),
        when: summary.and_then(|m| m.date).map(age),
    })
}

fn there_side(entry: &RemoteTagEntry) -> Option<Side> {
    let id = entry.peeled.as_ref().or(entry.remote_id.as_ref())?;
    let commit = entry.remote_commit.as_ref();
    let subject = match (&entry.symbolic_target, commit) {
        (Some(target), _) => crate::i18n::trf!("Follows {target}", target = display(target)),
        (None, Some(commit)) => display(&commit.subject),
        (None, None) => crate::i18n::tr("Commit not fetched").to_owned(),
    };
    Some(Side {
        sha: short(id).to_owned(),
        subject,
        when: commit.map(|c| age(c.date)),
    })
}

fn side_value(ui: &mut Ui, side: Option<&Side>, missing: &str) {
    let Some(side) = side else {
        ui.label(RichText::new(missing).color(MUTED()));
        return;
    };
    ui.label(RichText::new(&side.sha).monospace().size(12.0).color(MUTED()));
    if !side.subject.is_empty() {
        ui.add(Label::new(RichText::new(&side.subject)).wrap());
    }
    if let Some(when) = &side.when {
        ui.label(RichText::new(when).size(12.0).color(HINT()));
    }
}

/// Publish or Delete, whichever the tag allows. `ready` is whether they can start now.
fn remote_actions(
    ui: &mut Ui,
    entry: &RemoteTagEntry,
    widgets: &mut HashMap<String, UiControl>,
    remote: &str,
    ready: bool,
) -> Option<Ask> {
    let mut asked = None;
    let relation = entry.relation();
    if relation == "local_only" && entry.local.as_ref().is_some_and(|l| !l.symbolic) {
        let b = pages::primary(ui, ready, &crate::i18n::trf!("Publish to {remote}", remote));
        control(widgets, "dialog-remote-tag-publish", &b, ui.clip_rect());
        if b.clicked() {
            asked = Some(Ask::Review(RemoteTagAction::Publish));
        }
    }
    if matches!(relation, "published" | "different" | "remote_only") {
        let b = pages::danger_ghost(ui, ready, &crate::i18n::trf!("Delete from {remote}", remote));
        control(widgets, "dialog-remote-tag-delete", &b, ui.clip_rect());
        if b.clicked() {
            asked = Some(Ask::Review(RemoteTagAction::Delete));
        }
    }
    asked
}

/// The selected tag on both sides, and what can be done about it. A narrow window keeps
/// its actions in the bar above. Returns what was asked for.
fn remote_details(
    ui: &mut Ui,
    r: &RemoteState,
    widgets: &mut HashMap<String, UiControl>,
    remote: &str,
    ready: bool,
    narrow: bool,
) -> Option<Ask> {
    let Some(entry) = r.selected_entry() else {
        pages::empty(ui, crate::i18n::tr("Select a tag to compare it here and on the remote."));
        return None;
    };
    let mut asked = None;
    let name = display(&entry.name);
    let (kind, _, _) = remote_row_text(entry, remote);
    let (status, color) = relation(entry, remote);
    let relation = entry.relation();
    let (here, there) = (here_side(entry), there_side(entry));
    ui.spacing_mut().item_spacing.y = 10.0;
    if narrow {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            ui.add(Label::new(RichText::new(&name).font(semibold(16.0))).truncate());
            pages::pill(ui, &status, color);
            if let Some((kind, kind_color)) = kind {
                pages::pill(ui, kind, kind_color);
            }
        });
    } else {
        ui.add(Label::new(RichText::new(&name).font(semibold(18.0))).wrap());
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            pages::pill(ui, &status, color);
            if let Some((kind, kind_color)) = kind {
                pages::pill(ui, kind, kind_color);
            }
        });
    }
    let on_remote = crate::i18n::trf!("On {remote}", remote);
    if narrow {
        let both = [(crate::i18n::tr("Both"), here.as_ref(), "")];
        let sides = [(crate::i18n::tr("Here"), here.as_ref(), crate::i18n::tr("Not here")), (on_remote.as_str(), there.as_ref(), crate::i18n::tr("Not there yet"))];
        let shown: &[_] = if relation == "published" { &both } else { &sides };
        ui.columns(shown.len(), |columns| {
            for (ui, (title, side, missing)) in columns.iter_mut().zip(shown) {
                ui.spacing_mut().item_spacing.y = 2.0;
                ui.label(RichText::new(*title).size(12.0).color(HINT()));
                side_value(ui, *side, missing);
            }
        });
    } else {
        card(ui, crate::i18n::tr("Where it points"), |ui| {
            if relation == "published" {
                pages::key_value(ui, crate::i18n::tr("Both"), 76.0, |ui| side_value(ui, here.as_ref(), ""));
            } else {
                pages::key_value(ui, crate::i18n::tr("Here"), 76.0, |ui| side_value(ui, here.as_ref(), crate::i18n::tr("Not here")));
                pages::divider(ui);
                pages::key_value(ui, &on_remote, 76.0, |ui| side_value(ui, there.as_ref(), crate::i18n::tr("Not there yet")));
            }
        });
    }
    let note = match relation {
        "different" => Some((
            crate::i18n::trf!("Publishing won't replace {remote}'s tag. To replace it, delete it from {remote} first.", remote),
            AMBER(),
        )),
        "remote_only" => Some((crate::i18n::trf!("Fetch from {remote} to get this tag.", remote), MUTED())),
        "symbolic" => Some((
            crate::i18n::trf!(
                "This tag on {remote} points at {target}, so Rocket will not publish or delete it.",
                remote, target = entry.symbolic_target.as_deref().map(display).unwrap_or_default()
            ),
            AMBER(),
        )),
        _ => None,
    };
    if let Some((note, color)) = note {
        ui.add(Label::new(RichText::new(note).color(color)).wrap());
    }
    let commit = entry
        .summary
        .as_ref()
        .filter(|m| entry.local.is_some() && m.target_type == "commit")
        .and_then(|m| m.target.clone());
    if let Some(id) = commit {
        let b = ui.add(Button::new(RichText::new(crate::i18n::tr("Show in history")).size(12.5).color(BLUE())).frame(false));
        control(widgets, "dialog-remote-tag-show-commit", &b, ui.clip_rect());
        if b.clicked() {
            asked = Some(Ask::Show(id));
        }
    }
    if !narrow && relation != "symbolic" {
        pages::divider(ui);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if let Some(ask) = remote_actions(ui, entry, widgets, remote, ready) {
                    asked = Some(ask);
                }
            });
        });
    }
    asked
}
