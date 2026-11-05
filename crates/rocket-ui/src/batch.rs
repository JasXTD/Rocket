//! Several branches or tags at once. Choose them from a checklist, review, then
//! apply one at a time with each item's own checks; refusals and failures are
//! reported by name. Nothing is applied from a review that refused an item or no
//! longer matches the selection.
use crate::{
    app::*,
    backend::{Action, Outcome, user_error},
    pages::{self, control},
    style::*,
    tags::{self, DialogState, RemoteState},
};
use eframe::egui::*;
use rocket_git::*;
use std::{collections::HashMap, sync::Arc};

/// Names listed in a selection summary before "and N more".
const SUMMARY_NAMES: usize = 40;
const ROW_HEIGHT: f32 = 24.0;

/// One checklist row.
pub struct Item {
    pub name: Vec<u8>,
    pub label: String,
    pub detail: String,
    /// A tag's kind pill; tag rows show the detail on a second line.
    pub kind: Option<(&'static str, Color32)>,
}
/// Height of a two-line tag row in a checklist.
const TAG_ROW_HEIGHT: f32 = 40.0;

/// Tag changes chosen for one review: the names, then the review itself.
pub struct TagBatch<P> {
    pub selected: Vec<Vec<u8>>,
    pub filter: String,
    pub task: Option<ReadTask<TagBatchReview<P>>>,
    pub review: Option<Arc<TagBatchReview<P>>>,
}
impl<P> Default for TagBatch<P> {
    fn default() -> Self {
        Self {
            selected: Vec::new(),
            filter: String::new(),
            task: None,
            review: None,
        }
    }
}
impl<P> TagBatch<P> {
    pub fn running(&self) -> bool {
        self.task.is_some()
    }
    /// The review to apply: nothing refused, and it covers exactly the selection.
    pub fn ready(&self) -> Option<&Arc<TagBatchReview<P>>> {
        self.review
            .as_ref()
            .filter(|r| r.refused.is_empty() && r.ready.len() == self.selected.len())
    }
    fn invalidate(&mut self) {
        self.task = None;
        self.review = None;
    }
}

/// A local branch's name without `refs/heads/`.
pub fn branch_name(branch: &Branch) -> &[u8] {
    branch
        .reference
        .strip_prefix(b"refs/heads/")
        .unwrap_or(&branch.reference)
}

/// Local branches a batch can delete, in list order. Git refuses the current
/// branch and branches checked out in other worktrees, so those are not offered.
pub fn deletable_branches(branches: &[Branch]) -> impl Iterator<Item = &Branch> {
    branches.iter().filter(|b| {
        b.kind == BranchKind::Local
            && b.symbolic_target.is_none()
            && !b.is_head
            && b.checked_out_at.is_none()
    })
}

/// The rows the "Delete branches" dialog shows for this filter.
pub fn branch_items(branches: &[Branch], filter: &str) -> Vec<Item> {
    let needle = filter.to_lowercase();
    deletable_branches(branches)
        .map(|b| Item {
            name: branch_name(b).to_vec(),
            label: display(branch_name(b)),
            detail: match &b.upstream {
                Some(upstream) => format!("{} · tracks {}", short(&b.target), display(upstream)),
                None => short(&b.target).into(),
            },
            kind: None,
        })
        .filter(|item| item.label.to_lowercase().contains(&needle))
        .collect()
}

/// Local tags a deletion batch offers for its filter. Symbolic tags are inspect
/// only, so they are not offered.
pub fn delete_items(s: &DialogState, filter: &str) -> Vec<Item> {
    let needle = filter.to_lowercase();
    s.page
        .iter()
        .flat_map(|page| {
            page.entries
                .iter()
                .enumerate()
                .map(|(i, e)| (e, page.summaries.get(i)))
        })
        .filter(|(e, _)| !e.symbolic)
        .map(|(e, summary)| tag_item(&e.name, e, summary))
        .filter(|item| item.label.to_lowercase().contains(&needle))
        .collect()
}

/// A tag's checklist row: its kind, then the commit it names and that commit's subject.
fn tag_item(name: &[u8], entry: &TagEntry, summary: Option<&TagSummary>) -> Item {
    let commit = summary.and_then(|m| m.target.as_ref()).unwrap_or(&entry.id);
    let subject = summary.map_or_else(String::new, |m| display(&m.subject));
    Item {
        name: name.to_vec(),
        label: display(name),
        detail: if subject.is_empty() {
            tags::short(commit).into()
        } else {
            format!("{} · {subject}", tags::short(commit))
        },
        kind: Some(tags::kind(entry)),
    }
}

/// Local tags a publication batch offers for its filter: those the remote
/// snapshot shows as local only.
pub fn publish_items(r: &RemoteState, filter: &str) -> Vec<Item> {
    publish_items_with(r, None, filter)
}
/// `publish_items`, with what the local tag list shows about each tag.
fn publish_items_with(r: &RemoteState, page: Option<&TagPage>, filter: &str) -> Vec<Item> {
    let needle = filter.to_lowercase();
    r.snapshot
        .iter()
        .flat_map(|snapshot| &snapshot.entries)
        .filter(|e| e.relation() == "local_only")
        .filter_map(|e| {
            let local = e.local.as_ref()?;
            Some(tag_item(&e.name, local, tags::summary_in(page, &e.name)))
        })
        .filter(|item| item.label.to_lowercase().contains(&needle))
        .collect()
}

fn short(id: &ObjectId) -> &str {
    &id.as_str()[..9]
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// Select all and Clear for a checklist. Returns whether the selection changed.
fn select_buttons(
    ui: &mut Ui,
    key: &str,
    items: &[Item],
    selected: &mut Vec<Vec<u8>>,
    limit: usize,
    enabled: bool,
    widgets: &mut HashMap<String, UiControl>,
) -> bool {
    let mut changed = false;
    let r = ui.add_enabled(
        enabled && items.iter().any(|i| !selected.contains(&i.name)),
        Button::new("Select all").small(),
    );
    control(widgets, format!("dialog-{key}-all"), &r, ui.clip_rect());
    if r.clicked() {
        for item in items {
            if selected.len() < limit && !selected.contains(&item.name) {
                selected.push(item.name.clone());
            }
        }
        changed = true;
    }
    let r = ui.add_enabled(
        enabled && !selected.is_empty(),
        Button::new("Clear").small(),
    );
    control(widgets, format!("dialog-{key}-none"), &r, ui.clip_rect());
    if r.clicked() {
        selected.clear();
        changed = true;
    }
    selected.sort();
    changed
}

/// One row per item: a checkbox with the detail beside it, or for tags a two-line
/// row in a bordered list. Returns whether the selection changed.
fn checklist_rows(
    ui: &mut Ui,
    key: &str,
    items: &[Item],
    selected: &mut Vec<Vec<u8>>,
    limit: usize,
    enabled: bool,
    widgets: &mut HashMap<String, UiControl>,
) -> bool {
    let mut changed = false;
    let tags = items.iter().any(|i| i.kind.is_some());
    let mut rows = |ui: &mut Ui| {
        ScrollArea::vertical()
            .id_salt(format!("dialog-{key}-list"))
            .max_height(if tags { 260.0 } else { 220.0 })
            .auto_shrink([false, true])
            .show_rows(
                ui,
                if tags { TAG_ROW_HEIGHT } else { ROW_HEIGHT },
                items.len(),
                |ui, range| {
                    for i in range {
                        let item = &items[i];
                        let mut on = selected.contains(&item.name);
                        let can = enabled && (on || selected.len() < limit);
                        let r = if let Some(kind) = item.kind {
                            let r = tag_check_row(ui, item, kind, on, can);
                            if r.clicked() {
                                on = !on;
                            }
                            r
                        } else {
                            ui.horizontal(|ui| {
                                let r = ui.add_enabled(can, checkbox(&mut on, item.label.as_str()));
                                ui.label(RichText::new(&item.detail).size(12.0).color(FAINT()));
                                r
                            })
                            .inner
                        };
                        control(widgets, format!("dialog-{key}-{i}"), &r, ui.clip_rect());
                        if on != selected.contains(&item.name) {
                            if on {
                                selected.push(item.name.clone());
                            } else {
                                selected.retain(|n| n != &item.name);
                            }
                            changed = true;
                        }
                    }
                },
            )
    };
    let output = if tags {
        Frame::new()
            .stroke(Stroke::new(1.0, CARD_STROKE()))
            .corner_radius(8)
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                rows(ui)
            })
            .inner
    } else {
        rows(ui)
    };
    pages::region(
        widgets,
        &format!("dialog-{key}-scroll"),
        output.inner_rect.intersect(ui.clip_rect()),
        enabled,
    );
    selected.sort();
    changed
}

/// A tag in a checklist: a box, the name and kind, then what it points to. The whole
/// row toggles it.
fn tag_check_row(
    ui: &mut Ui,
    item: &Item,
    (kind, color): (&str, Color32),
    on: bool,
    enabled: bool,
) -> Response {
    let (rect, response) = ui
        .add_enabled_ui(enabled, |ui| {
            ui.allocate_exact_size(vec2(ui.available_width(), TAG_ROW_HEIGHT), Sense::click())
        })
        .inner;
    let painter = ui.painter();
    if enabled && (response.hovered() || response.has_focus()) {
        painter.rect_filled(rect, 0.0, HOVER());
    }
    painter.hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0, HAIRLINE()));
    let ink = if enabled { TEXT() } else { MUTED() };
    let mark = Rect::from_center_size(pos2(rect.left() + 19.0, rect.center().y), vec2(14.0, 14.0));
    if on {
        painter.rect_filled(mark, 3.0, if enabled { BLUE() } else { BLUE().gamma_multiply(0.5) });
        icon(painter, mark.shrink(2.0), Icon::Check, ACCENT_INK());
    } else {
        painter.rect(mark, 3.0, BG(), Stroke::new(1.0, BORDER()), StrokeKind::Inside);
    }
    let left = rect.left() + 38.0;
    let width = (rect.right() - 12.0 - left).max(40.0);
    let pill = painter.layout_no_wrap(kind.to_owned(), FontId::proportional(11.5), color);
    let pill_width = pill.size().x + 14.0;
    let mut job = text::LayoutJob::simple_singleline(item.label.clone(), semibold(13.0), ink);
    job.wrap = text::TextWrapping::truncate_at_width((width - pill_width - 8.0).max(20.0));
    let name = painter.layout_job(job);
    let line = rect.top() + 13.0;
    let pill_rect = Rect::from_min_size(
        pos2(left + name.size().x + 8.0, line - 8.5),
        vec2(pill_width, 17.0),
    );
    painter.galley(pos2(left, line - name.size().y / 2.0), name, ink);
    painter.rect_filled(pill_rect, 8.5, color.gamma_multiply(0.14));
    painter.galley(
        pos2(pill_rect.left() + 7.0, pill_rect.center().y - pill.size().y / 2.0),
        pill,
        color,
    );
    let mut job = text::LayoutJob::simple_singleline(item.detail.clone(), FontId::proportional(12.0), HINT());
    job.wrap = text::TextWrapping::truncate_at_width(width);
    let detail = painter.layout_job(job);
    painter.galley(pos2(left, rect.bottom() - 12.0 - detail.size().y / 2.0), detail, HINT());
    response.widget_info(|| WidgetInfo::selected(WidgetType::Checkbox, enabled, on, &item.label));
    response
}

impl DialogState {
    /// A tag batch review is open for the page being shown (local or remote).
    pub fn batch_open(&self) -> bool {
        if self.remote.active {
            self.remote.batch.is_some()
        } else {
            self.batch.is_some()
        }
    }
}

#[derive(Default)]
struct Flags {
    review: bool,
    submit: bool,
    back: bool,
    cancel: bool,
}

impl Rocket {
    /// Start reviewing the selected tags of the open batch.
    pub(crate) fn start_tag_batch_review(&mut self) {
        let Some(d) = &mut self.dialog else { return };
        let Some(s) = &mut d.tags else { return };
        if s.applying {
            return;
        }
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
        d.error = None;
        if s.remote.active {
            let snapshot = s.remote.snapshot.clone();
            let Some(batch) = &mut s.remote.batch else {
                return;
            };
            batch.invalidate();
            let Some(snapshot) = snapshot else {
                d.error = Some("Refresh the remote list first.".into());
                return;
            };
            let names = batch.selected.clone();
            match ReadTask::spawn(move |cancel| {
                backend.publish_tags_preview(&path, &snapshot, &names, &cancel)
            }) {
                Ok(task) => batch.task = Some(task),
                Err(e) => d.error = Some(user_error(&e)),
            }
        } else {
            let Some(batch) = &mut s.batch else { return };
            batch.invalidate();
            let entries = s
                .page
                .iter()
                .flat_map(|page| &page.entries)
                .filter(|e| batch.selected.contains(&e.name))
                .cloned()
                .collect::<Vec<_>>();
            match ReadTask::spawn(move |cancel| {
                backend.delete_tags_preview(&path, &entries, &cancel)
            }) {
                Ok(task) => batch.task = Some(task),
                Err(e) => d.error = Some(user_error(&e)),
            }
        }
    }

    pub(crate) fn poll_tag_batch(&mut self) {
        let Some(d) = &mut self.dialog else { return };
        let Some(s) = &mut d.tags else { return };
        if let Some(batch) = &mut s.batch
            && let Some(result) = batch.task.as_mut().and_then(ReadTask::try_finish)
        {
            batch.task = None;
            match result {
                Ok(review) => batch.review = Some(Arc::new(review)),
                Err(e) => d.error = Some(user_error(&e)),
            }
        }
        let r = &mut s.remote;
        if let Some(batch) = &mut r.batch
            && let Some(result) = batch.task.as_mut().and_then(ReadTask::try_finish)
        {
            batch.task = None;
            match result {
                Ok(review) => batch.review = Some(Arc::new(review)),
                Err(e) => {
                    // As with one tag: a failed remote read leaves the list stale.
                    r.stale = true;
                    d.error = Some(user_error(&e));
                }
            }
        }
    }

    /// Finish a batch write started from the batch dialog. Returns whether the
    /// completed write was one.
    pub(crate) fn finish_tag_batch(&mut self, owner: Option<u64>, outcome: &Outcome) -> bool {
        let Some(d) = &mut self.dialog else {
            return false;
        };
        let Some(s) = &mut d.tags else { return false };
        let remote = s.remote.active;
        if !s.applying
            || d.owner != owner
            || (remote && s.remote.batch.is_none())
            || (!remote && s.batch.is_none())
        {
            return false;
        }
        s.applying = false;
        let report = format!("{}\n{}", outcome.summary, outcome.details.join("\n"));
        if remote {
            // Like one tag: no automatic retry or extra network read after a write.
            let name = tags::remote_name(d);
            let Some(s) = &mut d.tags else { return false };
            let r = &mut s.remote;
            let review = r.batch.take().and_then(|b| b.review);
            r.preview = None;
            r.action = None;
            if let Some(review) = review.filter(|_| outcome.command_succeeded && !outcome.recovery_pending) {
                for preview in &review.ready {
                    r.applied(preview);
                }
                let count = plural(review.ready.len(), "tag", "tags");
                r.done = Some(format!("Published {count} to {name} and verified."));
                return true;
            }
            r.stale = true;
            r.notice = Some(format!("{report}\nRead again before another change."));
            if !outcome.command_succeeded {
                d.error = Some(outcome.summary.clone());
            }
            return true;
        }
        if outcome.command_succeeded && !outcome.recovery_pending {
            s.batch = None;
            s.read_list = true;
            self.start_tag_read();
        } else if let Some(batch) = &mut s.batch {
            batch.invalidate();
            d.error = Some(format!(
                "{report}\nReturn to the list and refresh before retrying."
            ));
        }
        true
    }

    /// Review dialog of a tag batch: deleting local tags from the Tags page, or
    /// publishing local tags from the Remote tags page.
    pub(crate) fn tag_batch_dialog(&mut self, ctx: &Context) {
        let writable = self.can_write_repository();
        let operation = pages::Running::of(self.write.as_ref());
        let tab = self.tab().map(|t| t.id);
        let Some(mut d) = self.dialog.take() else {
            return;
        };
        let Dialog {
            tags: Some(s),
            error,
            owner,
            remotes,
            remote: remote_index,
            ..
        } = &mut d
        else {
            self.dialog = Some(d);
            return;
        };
        let name = remotes
            .get(*remote_index)
            .map_or_else(|| "the remote".into(), |r| display(r));
        // As for one tag: a review that belongs to another tab is dropped.
        if *owner != tab && !s.applying {
            return;
        }
        let remote = s.remote.active;
        let applying = s.applying;
        let stale = remote && s.remote.stale;
        let mut f = Flags::default();
        let subtitle = if remote {
            format!("Sends the tags you pick to {name} so others can fetch them.")
        } else {
            "Removes the tags you pick from this repository. The commits stay.".into()
        };
        let head = pages::ReviewHeader {
            image: Icon::Tag,
            title: if remote { "Publish tags" } else { "Delete tags" },
            subtitle: &subtitle,
            danger: !remote,
        };
        let response = pages::review(
            ctx,
            "rocket-tag-batch",
            head,
            &mut self.frame_widgets,
            "dialog-tags-scroll",
            !applying,
            |ui, part, widgets| {
                let (running, has_review, ready, count) = if remote {
                    let b = s.remote.batch.get_or_insert_with(Default::default);
                    (
                        b.running(),
                        b.review.is_some(),
                        b.ready().is_some(),
                        b.selected.len(),
                    )
                } else {
                    let b = s.batch.get_or_insert_with(Default::default);
                    (
                        b.running(),
                        b.review.is_some(),
                        b.ready().is_some(),
                        b.selected.len(),
                    )
                };
                let waiting = running || applying;
                match part {
                    pages::Part::Body => {
                        let items = if remote {
                            let r = &s.remote;
                            publish_items_with(r, s.page.as_ref(), &r.batch.as_ref().unwrap().filter)
                        } else {
                            delete_items(s, &s.batch.as_ref().unwrap().filter)
                        };
                        let (filter, selected) = if remote {
                            let b = s.remote.batch.as_mut().unwrap();
                            (&mut b.filter, &mut b.selected)
                        } else {
                            let b = s.batch.as_mut().unwrap();
                            (&mut b.filter, &mut b.selected)
                        };
                        let mut changed = ui
                            .horizontal(|ui| {
                                ui.add_enabled_ui(!waiting, |ui| {
                                    let width = (ui.available_width() - 150.0).max(120.0);
                                    let r = search_field(ui, filter, "Filter tags", width);
                                    control(widgets, "dialog-batch-filter", &r, ui.clip_rect());
                                });
                                select_buttons(ui, "batch", &items, selected, MAX_TAG_BATCH, !waiting, widgets)
                            })
                            .inner;
                        changed |= checklist_rows(
                            ui,
                            "batch",
                            &items,
                            selected,
                            MAX_TAG_BATCH,
                            !waiting,
                            widgets,
                        );
                        if items.is_empty() {
                            pages::empty(ui, "No tags match the filter.");
                        }
                        pages::note(
                            ui,
                            &format!(
                                "At most {MAX_TAG_BATCH} at once{}",
                                if remote {
                                    format!(" · only tags {name} does not have are listed")
                                } else {
                                    String::new()
                                }
                            ),
                        );
                        if changed {
                            *error = None;
                            if remote {
                                s.remote.batch.as_mut().unwrap().invalidate();
                            } else {
                                s.batch.as_mut().unwrap().invalidate();
                            }
                        }
                        if remote {
                            let b = s.remote.batch.as_ref().unwrap();
                            if let Some(review) = &b.review {
                                let title = format!("Publishes {}", plural(review.ready.len(), "tag", "tags"));
                                let note = format!(" · to {name} · nothing there is replaced");
                                review_list(ui, review, (&title, &note), |p| {
                                    let entry = p.entry();
                                    let detail = match &entry.local {
                                        Some(local) => {
                                            let summary = tags::summary_in(s.page.as_ref(), &entry.name);
                                            tag_item(&entry.name, local, summary).detail
                                        }
                                        None => p.expected_result().map_or("?", tags::short).into(),
                                    };
                                    (display(&entry.name), detail)
                                });
                                if let Some(snapshot) = &s.remote.snapshot {
                                    pages::note(ui, &format!("To {}", snapshot.destination_display()));
                                }
                            }
                        } else {
                            let b = s.batch.as_ref().unwrap();
                            if let Some(review) = &b.review {
                                let title = format!("Deletes {}", plural(review.ready.len(), "tag", "tags"));
                                let note = " · each kept in refs/rocket/deleted-tags · commits stay";
                                review_list(ui, review, (&title, note), |p| {
                                    let details = p.details();
                                    let detail = match &details.commit {
                                        Some(commit) => format!("{} · {}", tags::short(&details.target), subject(commit)),
                                        None => tags::short(&details.target).into(),
                                    };
                                    (display(&details.entry.name), detail)
                                });
                            }
                        }
                        if has_review && !ready {
                            pages::callout(
                                ui,
                                AMBER(),
                                "Clear the refused tags, or refresh the list, then review again.",
                            );
                        }
                        // Changing the tags shows its status and Stop in the footer.
                        if waiting && !applying {
                            pages::waiting(ui, "Reviewing the selected tags…");
                        }
                        if let Some(text) = error.as_ref() {
                            pages::callout(ui, RED(), text);
                        }
                        if remote && let Some(notice) = &s.remote.notice {
                            pages::callout(ui, AMBER(), notice);
                        }
                    }
                    pages::Part::Footer if applying => {
                        f.cancel = pages::running_footer(ui, widgets, &operation, &[]);
                    }
                    pages::Part::Footer => {
                        let go = ready && !waiting && !stale && writable && error.is_none();
                        let label = if count == 0 { "tags".to_owned() } else { plural(count, "tag", "tags") };
                        let r = if remote {
                            pages::primary(ui, go, &format!("Publish {label}"))
                        } else {
                            pages::danger(ui, go, &format!("Delete {label}"))
                        };
                        control(widgets, "dialog-submit", &r, ui.clip_rect());
                        f.submit = r.clicked();
                        let can_review = !waiting && count > 0 && !stale;
                        let r = if has_review || error.is_some() {
                            ui.add_enabled(can_review, Button::new("Review again"))
                        } else {
                            pages::primary(ui, can_review, "Review")
                        };
                        control(widgets, "dialog-batch-review", &r, ui.clip_rect());
                        f.review = r.clicked();
                        let verb = if remote { "publish" } else { "delete" };
                        let status = if waiting {
                            "Reviewing".to_owned()
                        } else if error.is_some() || (has_review && !ready) {
                            "Fix the problem above, then review again".to_owned()
                        } else if count == 0 {
                            format!("Pick the tags to {verb}")
                        } else if ready {
                            format!("{count} selected · reviewed · nothing has changed yet")
                        } else {
                            format!("{count} selected · review before you {verb}")
                        };
                        pages::footer_status(ui, false, &status);
                    }
                }
            },
        );
        // The × returns to the page; automation's old Cancel id is that ×.
        if let Some(close) = self.frame_widgets.get("dialog-close").copied() {
            self.frame_widgets.insert("dialog-batch-back".into(), close);
        }
        f.back |= response.should_close() && !applying;
        if f.back {
            if remote {
                s.remote.batch = None;
            } else {
                s.batch = None;
            }
            *error = None;
        }
        let action = if !f.submit {
            None
        } else if remote {
            s.remote
                .batch
                .as_ref()
                .and_then(TagBatch::ready)
                .map(|review| Action::RemoteTagBatch(review.clone()))
        } else {
            s.batch
                .as_ref()
                .and_then(TagBatch::ready)
                .map(|review| Action::TagBatch(review.clone()))
        };
        let owner = *owner;
        if let (Some(owner), Some(action)) = (owner, action) {
            s.applying = true;
            self.execute(owner, action);
            if self.write.is_none() {
                s.applying = false;
                *error =
                    Some("Could not start. Wait for the repository, then review again.".into());
            }
        }
        self.dialog = Some(d);
        if f.cancel {
            self.cancel_write();
        }
        if f.review {
            self.start_tag_batch_review();
        }
    }
}

/// The reviewed tags and, in red, any the review refused with Git's reason.
fn review_list<P>(
    ui: &mut Ui,
    review: &TagBatchReview<P>,
    (title, note): (&str, &str),
    describe: impl Fn(&P) -> (String, String),
) {
    if !review.ready.is_empty() {
        pages::titled_card(
            ui,
            |ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                ui.label(RichText::new(title).strong());
                ui.label(RichText::new(note).size(12.0).color(HINT()));
            },
            |ui| {
                for p in review.ready.iter().take(SUMMARY_NAMES) {
                    let (name, detail) = describe(p);
                    pages::key_value(ui, &name, 160.0, |ui| {
                        ui.add(Label::new(RichText::new(detail).size(12.5).color(HINT())).truncate());
                    });
                }
                if review.ready.len() > SUMMARY_NAMES {
                    pages::note(
                        ui,
                        &format!("and {} more", review.ready.len() - SUMMARY_NAMES),
                    );
                }
            },
        );
    }
    for (name, error) in &review.refused {
        pages::callout(
            ui,
            RED(),
            &format!("{}: {}", display(name), user_error(error)),
        );
    }
}

/// Activity text for a branch batch: Git's own line for each deleted branch, and
/// the first line of Git's refusal for each kept one.
pub(crate) fn branch_batch_outcome(out: &mut Outcome, report: BranchBatchReport) {
    let total = report.results.len() + report.skipped.len();
    let mut deleted = 0;
    let mut kept = Vec::new();
    for (name, result) in report.results {
        match result.map(|r| r.write.result) {
            Ok(Ok(output)) => {
                deleted += 1;
                let text = String::from_utf8_lossy(&output.stdout);
                out.details.push(match text.lines().next() {
                    Some(line) if !line.trim().is_empty() => line.trim().to_owned(),
                    _ => format!("Deleted branch {}.", display(&name)),
                });
            }
            Ok(Err(error)) | Err(error) => {
                let reason = match &error {
                    Error::Git { stderr, .. } => String::from_utf8_lossy(stderr)
                        .lines()
                        .next()
                        .map(|l| l.trim().trim_start_matches("error: ").to_owned())
                        .filter(|l| !l.is_empty())
                        .unwrap_or_else(|| user_error(&error)),
                    _ => user_error(&error),
                };
                kept.push(format!("Kept {}: {reason}", display(&name)));
            }
        }
    }
    out.details.extend(kept.iter().cloned());
    if !report.skipped.is_empty() {
        out.details.push(format!(
            "Not attempted after cancellation: {}",
            report
                .skipped
                .iter()
                .map(|n| display(n))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    out.command_succeeded = deleted == total;
    out.summary = if deleted == total {
        format!(
            "Deleted {}; remote branches unchanged.",
            plural(deleted, "local branch", "local branches")
        )
    } else {
        let skipped = match report.skipped.len() {
            0 => String::new(),
            n => format!("; {n} not attempted after cancellation"),
        };
        format!(
            "Deleted {deleted} of {total} local branches; Git kept {}{skipped}. See the details.",
            kept.len()
        )
    };
}

/// Activity text for a tag batch: each verified change, then the first that was
/// not verified and the tags left unchanged after it.
pub(crate) fn tag_batch_outcome(out: &mut Outcome, report: TagBatchReport) {
    let total = report.results.len() + report.skipped.len();
    let mut verified = 0;
    for (name, result) in report.results {
        match result {
            Ok(r) if r.result.is_ok() && r.verification.is_ok() && r.failure.is_none() => {
                verified += 1;
                out.details.push(match &r.recovery_reference {
                    Some(reference) => {
                        format!("Deleted {}; object kept at {reference}.", display(&name))
                    }
                    None => format!("Deleted {}.", display(&name)),
                });
            }
            Ok(r) => {
                let mut tag = Outcome::default();
                tag.tag(r);
                out.details
                    .push(format!("{}: {}", display(&name), tag.summary));
                out.details.extend(tag.details);
                out.raw_output.push_str(&tag.raw_output);
            }
            Err(error) => out
                .details
                .push(format!("{}: {}", display(&name), user_error(&error))),
        }
    }
    if !report.skipped.is_empty() {
        out.details.push(format!(
            "Not changed: {}",
            report
                .skipped
                .iter()
                .map(|n| display(n))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    out.command_succeeded = verified == total;
    out.recovery_pending = !out.command_succeeded;
    out.summary = if out.command_succeeded {
        format!(
            "Deleted {} and verified {}; remote tags unchanged.",
            plural(verified, "local tag", "local tags"),
            if verified == 1 { "it" } else { "each" }
        )
    } else {
        format!(
            "Deleted {verified} of {total} local tags, then stopped. Inspect the tag that needs review before retrying."
        )
    };
}

/// Activity text for publishing several tags; like [`tag_batch_outcome`].
pub(crate) fn remote_tag_batch_outcome(out: &mut Outcome, report: RemoteTagBatchReport) {
    let total = report.results.len() + report.skipped.len();
    let mut verified = 0;
    for (name, result) in report.results {
        match result {
            Ok(r)
                if r.result.is_ok()
                    && r.observed.is_ok()
                    && r.verification.is_ok()
                    && !r.error_diagnostics =>
            {
                verified += 1;
                out.details.push(format!("Published {}.", display(&name)));
            }
            Ok(r) => {
                let mut tag = Outcome::default();
                tag.remote_tag(r);
                out.details
                    .push(format!("{}: {}", display(&name), tag.summary));
                out.details.extend(tag.details);
                out.raw_output.push_str(&tag.raw_output);
            }
            Err(error) => out
                .details
                .push(format!("{}: {}", display(&name), user_error(&error))),
        }
    }
    if !report.skipped.is_empty() {
        out.details.push(format!(
            "Not sent: {}",
            report
                .skipped
                .iter()
                .map(|n| display(n))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    out.command_succeeded = verified == total;
    out.recovery_pending = !out.command_succeeded;
    out.summary = if out.command_succeeded {
        format!(
            "Published {} and verified {} on the remote.",
            plural(verified, "tag", "tags"),
            if verified == 1 { "it" } else { "each" }
        )
    } else {
        format!(
            "Published {verified} of {total} tags, then stopped. Refresh the remote list and inspect the tag that needs review before retrying."
        )
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    fn branch(name: &str, head: bool, elsewhere: bool) -> Branch {
        Branch {
            reference: format!("refs/heads/{name}").into_bytes(),
            target: ObjectId::parse(&"a".repeat(40)).unwrap(),
            kind: BranchKind::Local,
            is_head: head,
            upstream: None,
            symbolic_target: None,
            checked_out_at: elsewhere.then(|| "C:/elsewhere".into()),
            committed: None,
            ahead_behind: None,
        }
    }

    #[test]
    fn only_branches_git_can_delete_are_offered_and_the_filter_ignores_case() {
        let mut remote = branch("origin/topic", false, false);
        remote.kind = BranchKind::Remote;
        remote.reference = b"refs/remotes/origin/topic".to_vec();
        let branches = [
            branch("main", true, true),
            branch("Feature/One", false, false),
            branch("other-worktree", false, true),
            branch("feature/two", false, false),
            remote,
        ];
        let names = |filter: &str| {
            branch_items(&branches, filter)
                .into_iter()
                .map(|i| i.label)
                .collect::<Vec<_>>()
        };
        assert_eq!(names(""), ["Feature/One", "feature/two"]);
        assert_eq!(names("FEATURE/T"), ["feature/two"]);
        assert!(names("main").is_empty());
    }
}

/// Click-driven tests of the real app frame; the helpers are shared with other
/// dialog tests.
#[cfg(test)]
pub(crate) mod flow_tests {
    use crate::{
        app::{DialogKind, Intent, Rocket},
        test_common as common,
    };
    use eframe::egui::{Context, Event, Modifiers, PointerButton, Pos2, RawInput, Rect, vec2};
    use std::{
        thread,
        time::{Duration, Instant},
    };

    const CASE: &str = "ROCKET_BATCH_UI_CASE";

    pub(crate) fn frame(ctx: &Context, app: &mut Rocket, events: Vec<Event>) {
        app.frame_widgets.clear();
        let mut output = ctx.run_ui(
            RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1480.0, 1000.0))),
                events,
                ..Default::default()
            },
            |ui| {
                // As in the app frame: the window, its intents, then dialogs.
                let mut intents = Vec::new();
                app.shell(ui, &mut intents);
                for intent in intents {
                    app.dispatch(intent);
                }
                app.dialog_ui(&ui.ctx().clone());
            },
        );
        output.textures_delta.clear();
    }
    pub(crate) fn enabled(ctx: &Context, app: &mut Rocket, key: &str) -> bool {
        // A newly shown dialog is laid out invisibly, without interaction, first.
        frame(ctx, app, vec![]);
        frame(ctx, app, vec![]);
        app.frame_widgets
            .get(key)
            .unwrap_or_else(|| panic!("{key} is not shown"))
            .enabled
    }
    pub(crate) fn click(ctx: &Context, app: &mut Rocket, key: &str) {
        assert!(enabled(ctx, app, key), "{key} is disabled");
        let pos = app.frame_widgets[key].rect.center();
        let press = |pressed| Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        frame(
            ctx,
            app,
            vec![Event::PointerMoved(pos), press(true), press(false)],
        );
        frame(ctx, app, vec![]);
    }
    /// Press on one control, move over a few frames and release over another.
    pub(crate) fn drag(ctx: &Context, app: &mut Rocket, from: &str, to: &str) {
        assert!(enabled(ctx, app, from), "{from} is disabled");
        let start = app.frame_widgets[from].rect.center();
        let end = app
            .frame_widgets
            .get(to)
            .unwrap_or_else(|| panic!("{to} is not shown"))
            .rect
            .center();
        let button = |pos, pressed| Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        frame(
            ctx,
            app,
            vec![Event::PointerMoved(start), button(start, true)],
        );
        for step in 1..=6 {
            let pos = start + (end - start) * (step as f32 / 6.0);
            frame(ctx, app, vec![Event::PointerMoved(pos)]);
        }
        frame(ctx, app, vec![button(end, false)]);
        frame(ctx, app, vec![]);
    }
    pub(crate) fn wait(ctx: &Context, app: &mut Rocket) {
        let deadline = Instant::now() + Duration::from_secs(60);
        while app.busy() {
            app.poll(ctx);
            assert!(Instant::now() < deadline, "work did not finish");
            thread::sleep(Duration::from_millis(10));
        }
        frame(ctx, app, vec![]);
    }
    pub(crate) fn open(ctx: &Context, f: &common::Fixture) -> Rocket {
        let mut app =
            Rocket::with_log_directory(ctx, vec![f.repository.clone()], f.root.join("logs"));
        wait(ctx, &mut app);
        app.auto_refresh = false;
        app
    }
    fn refs(f: &common::Fixture, git_dir: Option<&std::path::Path>, prefix: &str) -> Vec<String> {
        let dir = git_dir.map(|p| format!("--git-dir={}", p.display()));
        let mut arguments: Vec<&str> = dir.iter().map(String::as_str).collect();
        arguments.extend(["for-each-ref", "--format=%(refname:short)", prefix]);
        String::from_utf8(f.run(&arguments))
            .unwrap()
            .lines()
            .map(Into::into)
            .collect()
    }
    fn last_activity(app: &Rocket) -> (bool, String, Vec<String>) {
        let a = app.activities.last().expect("an activity");
        (
            a.outcome.command_succeeded,
            a.outcome.summary.clone(),
            a.outcome.details.clone(),
        )
    }

    #[test]
    fn delete_branches_keeps_what_git_refuses_and_reports_each_branch() {
        common::isolated(
            CASE,
            "batch::flow_tests::delete_branches_keeps_what_git_refuses_and_reports_each_branch",
            || {
                let f = common::Fixture::new();
                f.write("a.txt", "1\n");
                f.commit("base");
                for branch in ["merged-a", "merged-b", "unmerged"] {
                    f.run(&["branch", branch]);
                }
                f.run(&["switch", "unmerged"]);
                f.write("u.txt", "PRIVATE work\n");
                f.commit("unmerged work");
                f.run(&["switch", "main"]);
                let ctx = Context::default();
                let mut app = open(&ctx, &f);
                app.dispatch(Intent::Dialog(DialogKind::DeleteBranches));
                assert!(!enabled(&ctx, &mut app, "dialog-submit"));
                // The rows say what Git's deletion would accept.
                let deadline = Instant::now() + Duration::from_secs(60);
                while app.dialog.as_ref().unwrap().small.merge.is_none() {
                    assert!(Instant::now() < deadline, "merged check did not finish");
                    frame(&ctx, &mut app, vec![]);
                    thread::sleep(Duration::from_millis(20));
                }
                let merge = app.dialog.as_ref().unwrap().small.merge.clone().unwrap();
                let state = |name: &[u8]| merge.iter().find(|s| s.name == name).cloned().unwrap();
                assert!(state(b"merged-a").merged && state(b"merged-b").merged);
                assert_eq!((state(b"unmerged").merged, state(b"unmerged").unmerged), (false, Some(1)));
                click(&ctx, &mut app, "dialog-branches-merged");
                assert_eq!(
                    app.dialog.as_ref().unwrap().selected_branches,
                    [b"merged-a".to_vec(), b"merged-b".to_vec()]
                );
                // Rows: merged-a, merged-b, unmerged. Git keeps the unmerged one.
                click(&ctx, &mut app, "dialog-branches-2");
                // The current branch is never offered.
                assert_eq!(
                    app.dialog.as_ref().unwrap().selected_branches,
                    [
                        b"merged-a".to_vec(),
                        b"merged-b".to_vec(),
                        b"unmerged".to_vec()
                    ]
                );
                click(&ctx, &mut app, "dialog-submit");
                wait(&ctx, &mut app);
                assert_eq!(refs(&f, None, "refs/heads"), ["main", "unmerged"]);
                let (succeeded, summary, details) = last_activity(&app);
                assert!(!succeeded);
                assert!(
                    summary.starts_with("Deleted 2 of 3 local branches"),
                    "{summary}"
                );
                assert!(
                    details
                        .iter()
                        .any(|d| d.starts_with("Deleted branch merged-a (was ")),
                    "{details:?}"
                );
                assert!(
                    details
                        .iter()
                        .any(|d| d.starts_with("Kept unmerged:") && d.contains("not fully merged")),
                    "{details:?}"
                );
            },
        );
    }

    #[test]
    fn delete_several_tags_needs_a_current_review_and_refreshes_the_list() {
        common::isolated(
            CASE,
            "batch::flow_tests::delete_several_tags_needs_a_current_review_and_refreshes_the_list",
            || {
                let f = common::Fixture::new();
                f.write("a.txt", "1\n");
                f.commit("base");
                f.run(&["tag", "v1"]);
                f.run(&["tag", "--no-sign", "-a", "v2", "-m", "PRIVATE note"]);
                f.run(&["tag", "keep"]);
                f.run(&["symbolic-ref", "refs/tags/indirect", "refs/heads/main"]);
                let ctx = Context::default();
                let mut app = open(&ctx, &f);
                app.dispatch(Intent::Dialog(DialogKind::Tags));
                wait(&ctx, &mut app);
                click(&ctx, &mut app, "dialog-tags-batch");
                // Rows: keep, v1, v2 (the symbolic tag is not offered).
                click(&ctx, &mut app, "dialog-batch-1");
                click(&ctx, &mut app, "dialog-batch-2");
                assert!(!enabled(&ctx, &mut app, "dialog-submit"));
                click(&ctx, &mut app, "dialog-batch-review");
                wait(&ctx, &mut app);
                assert!(enabled(&ctx, &mut app, "dialog-submit"));
                // Changing the selection discards the review.
                click(&ctx, &mut app, "dialog-batch-0");
                assert!(!enabled(&ctx, &mut app, "dialog-submit"));
                click(&ctx, &mut app, "dialog-batch-0");
                click(&ctx, &mut app, "dialog-batch-review");
                wait(&ctx, &mut app);
                let tags = app.dialog.as_ref().unwrap().tags.as_ref().unwrap();
                let review = tags.batch.as_ref().unwrap().review.as_ref().unwrap();
                assert_eq!(review.ready.len(), 2);
                assert!(review.refused.is_empty());
                click(&ctx, &mut app, "dialog-submit");
                wait(&ctx, &mut app);
                assert_eq!(refs(&f, None, "refs/tags"), ["indirect", "keep"]);
                let (succeeded, summary, details) = last_activity(&app);
                assert!(succeeded, "{summary} {details:?}");
                assert_eq!(
                    summary,
                    "Deleted 2 local tags and verified each; remote tags unchanged."
                );
                assert!(
                    details.iter().all(|d| d.contains("object kept at")),
                    "{details:?}"
                );
                let tags = app.dialog.as_ref().unwrap().tags.as_ref().unwrap();
                assert!(tags.batch.is_none());
                let names: Vec<&[u8]> = tags
                    .page
                    .as_ref()
                    .unwrap()
                    .entries
                    .iter()
                    .map(|e| e.name.as_slice())
                    .collect();
                assert_eq!(names, [b"indirect".as_slice(), b"keep"]);
            },
        );
    }

    #[test]
    fn publish_several_tags_offers_only_unpublished_tags() {
        common::isolated(
            CASE,
            "batch::flow_tests::publish_several_tags_offers_only_unpublished_tags",
            || {
                let f = common::Fixture::new();
                f.write("a.txt", "1\n");
                f.commit("base");
                let remote = f.root.join("PRIVATE remote 文.git");
                f.run(&["init", "--bare", remote.to_str().unwrap()]);
                f.run(&["remote", "add", "origin", remote.to_str().unwrap()]);
                for tag in ["v1", "v2", "v3"] {
                    f.run(&["tag", tag]);
                }
                f.run(&["push", "origin", "refs/tags/v3:refs/tags/v3"]);
                let ctx = Context::default();
                let mut app = open(&ctx, &f);
                app.dispatch(Intent::Dialog(DialogKind::Tags));
                wait(&ctx, &mut app);
                click(&ctx, &mut app, "dialog-tags-remote");
                click(&ctx, &mut app, "dialog-remote-tag-refresh");
                wait(&ctx, &mut app);
                click(&ctx, &mut app, "dialog-remote-tags-batch");
                click(&ctx, &mut app, "dialog-batch-all");
                let tags = app.dialog.as_ref().unwrap().tags.as_ref().unwrap();
                assert_eq!(
                    tags.remote.batch.as_ref().unwrap().selected,
                    [b"v1".to_vec(), b"v2".to_vec()]
                );
                click(&ctx, &mut app, "dialog-batch-review");
                wait(&ctx, &mut app);
                click(&ctx, &mut app, "dialog-submit");
                wait(&ctx, &mut app);
                assert_eq!(refs(&f, Some(&remote), "refs/tags"), ["v1", "v2", "v3"]);
                let (succeeded, summary, _) = last_activity(&app);
                assert!(succeeded, "{summary}");
                assert_eq!(summary, "Published 2 tags and verified each on the remote.");
                let tags = app.dialog.as_ref().unwrap().tags.as_ref().unwrap();
                assert!(tags.remote.batch.is_none() && !tags.remote.stale && tags.remote.done.is_some());
            },
        );
    }
}
