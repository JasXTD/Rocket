//! Apply part of a stash: whole files, changes or single lines chosen from an immutable
//! review. Each change of the selection is checked at once; only a checked selection is
//! applied, through the normal write bridge.
use crate::{
    app::*,
    backend::{Action, user_error},
    pages::{self, control},
    style::*,
    workflows,
};
use eframe::egui::*;
use rocket_git::*;
use std::collections::{BTreeMap, BTreeSet, HashMap};

#[derive(Default)]
pub struct State {
    pub review: Option<StashApplyReview>,
    /// None = whole file; Some(set) = exact raw changed-line IDs.
    pub chosen: BTreeMap<usize, Option<BTreeSet<usize>>>,
    pub task: Option<ReadTask<StashApplyReview>>,
    /// The file whose changes the right side shows.
    pub file: usize,
}
impl State {
    pub fn selection(&self) -> Vec<StashApplySelection> {
        self.chosen
            .iter()
            .filter(|(_, h)| h.as_ref().is_none_or(|v| !v.is_empty()))
            .map(|(&file, lines)| StashApplySelection {
                file,
                hunks: Vec::new(),
                lines: lines
                    .as_ref()
                    .map(|v| v.iter().copied().collect())
                    .unwrap_or_default(),
            })
            .collect()
    }
}

/// What is chosen: per file, None for the whole file or the exact changed-line IDs.
type Chosen = BTreeMap<usize, Option<BTreeSet<usize>>>;

fn chosen_line(chosen: &Chosen, file: usize, id: usize) -> bool {
    match chosen.get(&file) {
        Some(None) => true,
        Some(Some(set)) => set.contains(&id),
        None => false,
    }
}

/// Adds or removes changed lines of a file (`all` are its changed lines); a file left with
/// none is no longer chosen.
fn choose_lines(chosen: &mut Chosen, file: usize, all: &[usize], ids: &[usize], on: bool) {
    let entry = chosen.entry(file).or_insert_with(|| Some(BTreeSet::new()));
    let set = entry.get_or_insert_with(|| all.iter().copied().collect());
    if on {
        set.extend(ids.iter().copied());
    } else {
        for id in ids {
            set.remove(id);
        }
    }
    if set.is_empty() {
        chosen.remove(&file);
    }
}

/// One row of a change: its raw patch line (the selection ID), its old and new line
/// numbers, its sign and its text.
struct Row<'a> {
    id: usize,
    old: Option<u32>,
    new: Option<u32>,
    sign: u8,
    text: &'a [u8],
}
/// One change (hunk): its lines on the new side, Git's context text and its rows.
struct Change<'a> {
    first: u32,
    last: u32,
    context: String,
    rows: Vec<Row<'a>>,
}

/// A file's saved patch as numbered changes. Raw line positions stay the selection IDs.
fn changes(patch: &[u8]) -> Vec<Change<'_>> {
    let mut out: Vec<Change<'_>> = Vec::new();
    let (mut old, mut new) = (0u32, 0u32);
    for (id, line) in patch.split(|b| *b == b'\n').enumerate() {
        if line.starts_with(b"@@") {
            // @@ -a,b +c,d @@ context
            let text = String::from_utf8_lossy(line);
            let mut parts = text.splitn(3, "@@");
            let ranges = parts.nth(1).unwrap_or("");
            let side = |sign: char| {
                let range = ranges.split_whitespace().find(|p| p.starts_with(sign)).map(|p| &p[1..]).unwrap_or("1");
                let mut numbers = range.split(',').map(|n| n.parse::<u32>().unwrap_or(1));
                (numbers.next().unwrap_or(1), numbers.next().unwrap_or(1))
            };
            let ((from, _), (to, count)) = (side('-'), side('+'));
            (old, new) = (from, to);
            out.push(Change {
                first: to,
                last: to + count.saturating_sub(1),
                context: parts.next().unwrap_or("").trim().to_owned(),
                rows: Vec::new(),
            });
            continue;
        }
        // File headers come before the first change.
        let Some(change) = out.last_mut() else { continue };
        let row = |sign, old, new| Row { id, old, new, sign, text: &line[1..] };
        match line.first() {
            Some(b'+') => {
                change.rows.push(row(b'+', None, Some(new)));
                new += 1;
            }
            Some(b'-') => {
                change.rows.push(row(b'-', Some(old), None));
                old += 1;
            }
            Some(b' ') => {
                change.rows.push(row(b' ', Some(old), Some(new)));
                old += 1;
                new += 1;
            }
            // "\ No newline at end of file" and the patch's final empty line.
            _ => (),
        }
    }
    out
}

/// Added and removed lines in a file's saved patch.
fn counts(patch: &[u8]) -> (usize, usize) {
    changes(patch).iter().flat_map(|c| &c.rows).fold((0, 0), |(add, del), r| match r.sign {
        b'+' => (add + 1, del),
        b'-' => (add, del + 1),
        _ => (add, del),
    })
}

/// Rows drawn per file; a longer file is still applied in full.
const SHOWN_ROWS: usize = 600;
const ROW: f32 = 20.0;

/// A checkbox registered for automation, with no label of its own.
fn tick(ui: &mut Ui, widgets: &mut HashMap<String, UiControl>, key: String, on: bool, mixed: bool, enabled: bool) -> bool {
    let mut value = on;
    let r = ui.add_enabled(enabled, Checkbox::without_text(&mut value).indeterminate(mixed && !on));
    control(widgets, key, &r, ui.clip_rect());
    r.changed()
}

impl Rocket {
    pub(crate) fn open_stash_apply(&mut self, entry: StashEntry, part: StashPart) {
        if !self.can_write() || self.dialog.is_some() {
            return;
        }
        let Some(repo) = self
            .tab()
            .and_then(|t| t.snapshot.as_ref())
            .map(|s| s.repository.clone())
        else {
            return;
        };
        let Some(backend) = self.backend.clone() else {
            return;
        };
        self.show_dialog(DialogKind::Workflows);
        let Some(d) = &mut self.dialog else {
            return;
        };
        let Some(s) = &mut d.workflow else {
            return;
        };
        s.mode = 15;
        s.stash_apply = Some(State::default());
        match ReadTask::spawn(move |c| {
            backend.log.request("stash-apply-preview", |log| {
                repo.with_log(log.clone())
                    .review_stash_apply(&entry, part, &c)
            })
        }) {
            Ok(task) => s.stash_apply.as_mut().unwrap().task = Some(task),
            Err(e) => d.error = Some(user_error(&e)),
        }
    }

    pub(crate) fn stash_apply_dialog(&mut self, ctx: &Context) {
        let writable = self.can_write();
        let Some(mut d) = self.dialog.take() else {
            return;
        };
        let Some(s) = &mut d.workflow else {
            self.dialog = Some(d);
            return;
        };
        if d.owner != self.tab().map(|t| t.id) && !s.applying {
            return;
        }
        let busy = s.running();
        let checking = s.task.is_some();
        let Some(saved) = &mut s.stash_apply else {
            self.dialog = Some(d);
            return;
        };
        let mut changed = false;
        let mut close = false;
        let mut again = false;
        let mut apply = false;
        let mut cancel = false;
        let running = pages::Running::of(self.write.as_ref());
        let subtitle = saved.review.as_ref().map_or_else(
            || "Reading the stash".to_owned(),
            |r| {
                let part = match r.part() {
                    StashPart::Tracked => "All tracked",
                    StashPart::Index => "Saved index",
                    StashPart::Untracked => "Untracked files",
                };
                format!("stash@{{{}}} · {} · {part}", r.entry().index, String::from_utf8_lossy(&r.entry().subject))
            },
        );
        let header = pages::ReviewHeader { image: Icon::Stash, title: "Apply part of a stash", subtitle: &subtitle, danger: false };
        let response = pages::split_review(
            ctx,
            "rocket-stash-apply",
            header,
            &mut self.frame_widgets,
            "dialog-stash-apply-scroll",
            true,
            |ui, part, widgets| match part {
                pages::Part::Body => {
                    if let Some(error) = &d.error {
                        pages::callout(ui, RED(), error);
                    }
                    if let Some(note) = &s.notice {
                        pages::callout(ui, if d.error.is_none() { GREEN() } else { AMBER() }, note);
                    }
                    let Some(review) = &saved.review else {
                        if saved.task.is_some() {
                            pages::waiting(ui, "Reading saved files and changes…");
                        }
                        return;
                    };
                    if review.files().is_empty() {
                        pages::empty(ui, "No saved changes in this part of the stash.");
                        return;
                    }
                    saved.file = saved.file.min(review.files().len() - 1);
                    let height = (ui.ctx().content_rect().height() - 330.0).clamp(200.0, 560.0);
                    let list = (ui.available_width() * 0.3).clamp(190.0, 270.0);
                    ui.horizontal_top(|ui| {
                        // The panes and the line between them touch; inside each the usual gaps apply.
                        let gap = ui.spacing().item_spacing.x;
                        ui.spacing_mut().item_spacing.x = 0.0;
                        // Files: a checkbox each (some of it, all of it, none) and its counts.
                        ui.allocate_ui_with_layout(vec2(list, height), Layout::top_down(Align::Min), |ui| {
                            ui.spacing_mut().item_spacing.x = gap;
                            ui.set_min_size(vec2(list, height));
                            ui.horizontal(|ui| {
                                let files = review.files().len();
                                ui.label(RichText::new(format!("{files} file{}", if files == 1 { "" } else { "s" })).size(12.0).color(HINT()));
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    let r = ui.add_enabled(!busy && !saved.chosen.is_empty(), Button::new("Clear").small().frame(false));
                                    control(widgets, "dialog-stash-none", &r, ui.clip_rect());
                                    if r.clicked() {
                                        saved.chosen.clear();
                                        changed = true;
                                    }
                                    let r = ui.add_enabled(!busy, Button::new("Select all").small().frame(false));
                                    control(widgets, "dialog-stash-all", &r, ui.clip_rect());
                                    if r.clicked() {
                                        for (i, file) in review.files().iter().enumerate() {
                                            if !file.mode_change() {
                                                saved.chosen.insert(i, None);
                                            }
                                        }
                                        changed = true;
                                    }
                                });
                            });
                            ScrollArea::vertical().id_salt("stash-apply-files").auto_shrink([false, false]).show(ui, |ui| {
                                for (i, file) in review.files().iter().enumerate() {
                                    let all = file.changed_line_ids();
                                    let (on, mixed) = match saved.chosen.get(&i) {
                                        Some(None) => (true, false),
                                        Some(Some(set)) => (all.iter().all(|id| set.contains(id)), !set.is_empty()),
                                        None => (false, false),
                                    };
                                    let shown = saved.file == i;
                                    let frame = Frame::new()
                                        .fill(if shown { SELECTED() } else { Color32::TRANSPARENT })
                                        .corner_radius(6)
                                        .inner_margin(Margin::symmetric(8, 6));
                                    frame.show(ui, |ui| {
                                        ui.set_width(ui.available_width());
                                        ui.horizontal(|ui| {
                                            ui.spacing_mut().item_spacing.x = 8.0;
                                            if tick(ui, widgets, format!("dialog-stash-file-{i}"), on, mixed, !busy && !file.mode_change()) {
                                                if on || mixed {
                                                    saved.chosen.remove(&i);
                                                } else {
                                                    saved.chosen.insert(i, None);
                                                }
                                                changed = true;
                                            }
                                            let path = file.path().display_lossy();
                                            let (dir, name) = path.rsplit_once('/').map_or(("", path.as_ref()), |(d, n)| (d, n));
                                            let (add, del) = counts(file.patch());
                                            let tail = if file.mode_change() {
                                                "mode".to_owned()
                                            } else if file.whole_only() {
                                                "whole".to_owned()
                                            } else {
                                                format!("+{add} −{del}")
                                            };
                                            let tail_width = ui.painter().layout_no_wrap(tail.clone(), FontId::proportional(11.5), MUTED()).size().x + 8.0;
                                            // Name and folder fill the room left of the counts, and all of it shows the file.
                                            let room = (ui.available_width() - tail_width).max(40.0);
                                            let mut job = text::LayoutJob { wrap: text::TextWrapping::truncate_at_width(room), ..Default::default() };
                                            job.append(name, 0.0, TextFormat::simple(semibold(13.0), TEXT()));
                                            if !dir.is_empty() {
                                                job.append(&format!("  {dir}/"), 0.0, TextFormat::simple(FontId::proportional(12.0), HINT()));
                                            }
                                            let (rect, label) = ui.allocate_exact_size(vec2(room, 20.0), Sense::click());
                                            let galley = ui.painter().layout_job(job);
                                            ui.painter().galley(pos2(rect.left(), rect.center().y - galley.size().y / 2.0), galley, TEXT());
                                            let label = if let Some(old) = file.original_path() {
                                                label.on_hover_text(format!("{} → {}", old.display_lossy(), path))
                                            } else {
                                                label.on_hover_text(path.as_ref())
                                            }
                                            .on_hover_cursor(CursorIcon::PointingHand);
                                            control(widgets, format!("dialog-stash-show-{i}"), &label, ui.clip_rect());
                                            if label.clicked() {
                                                saved.file = i;
                                            }
                                            let color = if file.whole_only() || file.mode_change() { HINT() } else { MUTED() };
                                            ui.label(RichText::new(tail).size(11.5).color(color));
                                        });
                                    });
                                }
                            });
                        });
                        let (line, _) = ui.allocate_exact_size(vec2(1.0, height), Sense::hover());
                        ui.painter().rect_filled(line, 0.0, CARD_LINE());
                        // The shown file's changes, as numbered cards with a box per changed line.
                        ui.allocate_ui_with_layout(vec2(ui.available_width(), height), Layout::top_down(Align::Min), |ui| {
                            ui.spacing_mut().item_spacing.x = gap;
                            let i = saved.file;
                            let file = &review.files()[i];
                            Frame::new().inner_margin(Margin { left: 16, right: 4, top: 0, bottom: 0 }).show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                let all = file.changed_line_ids();
                                let rows = changes(file.patch());
                                ui.horizontal(|ui| {
                                    let path = file.path().display_lossy();
                                    ui.label(RichText::new(path.as_ref()).monospace().strong());
                                    let picked = all.iter().filter(|id| chosen_line(&saved.chosen, i, **id)).count();
                                    ui.label(RichText::new(format!("{picked} of {} lines", all.len())).size(12.0).color(HINT()));
                                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                        let whole = matches!(saved.chosen.get(&i), Some(None));
                                        let r = ui.add_enabled(!busy && !file.mode_change() && !whole, Button::new("Use whole file").small());
                                        control(widgets, "dialog-stash-whole", &r, ui.clip_rect());
                                        if r.clicked() {
                                            saved.chosen.insert(i, None);
                                            changed = true;
                                        }
                                    });
                                });
                                ui.add_space(4.0);
                                ScrollArea::vertical().id_salt(("stash-apply-changes", i)).auto_shrink([false, false]).show(ui, |ui| {
                                    if file.mode_change() {
                                        pages::note(ui, "Executable-mode changes need the whole stash: use Apply on the Stashes page.");
                                        return;
                                    }
                                    let partial = !file.whole_only();
                                    if !partial {
                                        pages::note(ui, "New, deleted, renamed and binary files apply whole.");
                                    }
                                    let mut left = SHOWN_ROWS;
                                    for (h, change) in rows.iter().enumerate() {
                                        if left == 0 {
                                            pages::note(ui, "The rest of this file isn't shown here; choose whole changes or the whole file for it.");
                                            break;
                                        }
                                        let ids: Vec<usize> = change.rows.iter().filter(|r| r.sign != b' ').map(|r| r.id).collect();
                                        let picked = ids.iter().filter(|id| chosen_line(&saved.chosen, i, **id)).count();
                                        Frame::new().fill(DEEP()).stroke(Stroke::new(1.0, CARD_LINE())).corner_radius(8).show(ui, |ui| {
                                            ui.set_width(ui.available_width());
                                            // Rows meet like a diff's; the header keeps the card's top corners.
                                            ui.spacing_mut().item_spacing.y = 0.0;
                                            let top = CornerRadius { nw: 8, ne: 8, sw: 0, se: 0 };
                                            Frame::new().fill(PANEL()).corner_radius(top).inner_margin(Margin::symmetric(10, 6)).show(ui, |ui| {
                                                ui.set_width(ui.available_width());
                                                ui.horizontal(|ui| {
                                                    ui.spacing_mut().item_spacing.x = 8.0;
                                                    if partial && tick(ui, widgets, format!("dialog-stash-hunk-{i}-{h}"), picked == ids.len() && !ids.is_empty(), picked > 0, !busy) {
                                                        choose_lines(&mut saved.chosen, i, &all, &ids, picked < ids.len());
                                                        changed = true;
                                                    }
                                                    ui.label(RichText::new(format!("Change {}", h + 1)).font(semibold(12.5)));
                                                    let lines = if change.first == change.last { format!("line {}", change.first) } else { format!("lines {}–{}", change.first, change.last) };
                                                    let context = if change.context.is_empty() { lines } else { format!("{lines} · {}", change.context) };
                                                    let count = format!("{picked} of {} lines", ids.len());
                                                    let room = ui.available_width() - ui.painter().layout_no_wrap(count.clone(), FontId::proportional(12.0), HINT()).size().x - 12.0;
                                                    ui.allocate_ui(vec2(room.max(40.0), 18.0), |ui| {
                                                        // Take all the room, so the count sits at the right edge.
                                                        ui.set_min_width(room.max(40.0));
                                                        ui.add(Label::new(RichText::new(context).monospace().size(11.5).color(HINT())).truncate());
                                                    });
                                                    ui.label(RichText::new(count).size(12.0).color(HINT()));
                                                });
                                            });
                                            for row in change.rows.iter().take(left) {
                                                left -= 1;
                                                let changed_line = row.sign != b' ';
                                                let on = changed_line && chosen_line(&saved.chosen, i, row.id);
                                                let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), ROW), Sense::hover());
                                                let tint = match row.sign {
                                                    b'+' => GREEN().gamma_multiply(if on || !partial { 0.12 } else { 0.05 }),
                                                    b'-' => RED().gamma_multiply(if on || !partial { 0.12 } else { 0.05 }),
                                                    _ => Color32::TRANSPARENT,
                                                };
                                                ui.painter().rect_filled(rect, 0.0, tint);
                                                if partial && changed_line {
                                                    // A child at the box: unlike `put`, it leaves the row its fixed height.
                                                    let at = Rect::from_center_size(pos2(rect.left() + 14.0, rect.center().y), vec2(18.0, ROW));
                                                    let mut cell = ui.new_child(UiBuilder::new().max_rect(at).layout(Layout::centered_and_justified(Direction::TopDown)));
                                                    let mut value = on;
                                                    let r = cell.add_enabled(!busy, Checkbox::without_text(&mut value));
                                                    control(widgets, format!("dialog-stash-line-{i}-{}", row.id), &r, ui.clip_rect());
                                                    if r.changed() {
                                                        choose_lines(&mut saved.chosen, i, &all, &[row.id], !on);
                                                        changed = true;
                                                    }
                                                }
                                                let font = FontId::monospace(11.5);
                                                let ink = if changed_line && partial && !on { HINT() } else { TEXT() };
                                                let y = rect.center().y;
                                                let number = |n: Option<u32>, x: f32| {
                                                    if let Some(n) = n {
                                                        ui.painter().text(pos2(x, y), Align2::RIGHT_CENTER, n.to_string(), font.clone(), FAINT());
                                                    }
                                                };
                                                number(row.old, rect.left() + 64.0);
                                                number(row.new, rect.left() + 102.0);
                                                let sign = match row.sign {
                                                    b'+' => "+",
                                                    b'-' => "−",
                                                    _ => " ",
                                                };
                                                ui.painter().text(pos2(rect.left() + 114.0, y), Align2::CENTER_CENTER, sign, font.clone(), ink);
                                                let text = String::from_utf8_lossy(crate::text::prefix(row.text, 1024));
                                                let body = Rect::from_min_max(pos2(rect.left() + 126.0, rect.top()), rect.max);
                                                ui.painter().with_clip_rect(body.intersect(ui.clip_rect())).text(pos2(body.left(), y), Align2::LEFT_CENTER, text, font, ink);
                                            }
                                            ui.add_space(6.0);
                                        });
                                        ui.add_space(8.0);
                                    }
                                });
                            });
                        });
                    });
                }
                pages::Part::Footer if s.applying => {
                    cancel = pages::running_footer(ui, widgets, &running, &[]);
                }
                pages::Part::Footer => {
                    let selection = !saved.selection().is_empty();
                    let r = pages::primary(
                        ui,
                        writable && s.preview.is_some() && d.error.is_none() && !changed,
                        "Apply selected changes",
                    );
                    control(widgets, "dialog-submit", &r, ui.clip_rect());
                    apply = r.clicked();
                    if busy {
                        let r = ui.add_enabled(!s.stopping, Button::new(if s.stopping { "Stopping" } else { "Stop" }));
                        control(widgets, "dialog-stash-stop", &r, ui.clip_rect());
                        cancel = r.clicked();
                    } else if selection && s.preview.is_none() {
                        // After a refused or stopped check; changes are checked as they are made.
                        let r = ui.button("Check again");
                        control(widgets, "dialog-preview-refresh", &r, ui.clip_rect());
                        again = r.clicked();
                    }
                    if let Some(workflows::Review::StashApply(p)) = s.preview.as_deref().filter(|_| !busy) {
                        ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                            ui.spacing_mut().item_spacing.x = 6.0;
                            let (rect, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
                            icon(ui.painter(), rect, Icon::Check, GREEN());
                            let files = p.files().len();
                            let text = format!("Applies cleanly to {files} file{} · the stash and your staging stay as they are", if files == 1 { "" } else { "s" });
                            ui.add(Label::new(RichText::new(text).size(12.5).color(GREEN())).truncate());
                        });
                    } else {
                        let status = if s.stopping {
                            "Stopping"
                        } else if saved.task.is_some() {
                            "Reading the stash"
                        } else if checking {
                            "Checking the selection"
                        } else if d.error.is_some() {
                            "This selection can't be applied as it is"
                        } else if !selection {
                            "Choose files, changes or lines"
                        } else {
                            "Not checked yet"
                        };
                        pages::footer_status(ui, busy, status);
                    }
                }
            },
        );
        close |= response.should_close() && !s.applying;
        if changed {
            // A running check describes the old selection: drop it, and check again below.
            if let Some(task) = s.task.take() {
                task.cancel();
            }
            s.preview = None;
            s.notice = None;
            d.error = None;
        }
        let recheck = again || (changed && !saved.selection().is_empty());
        if cancel {
            if s.applying {
                self.cancel_write();
            }
            if let Some(t) = &s.task {
                t.cancel();
            }
            if let Some(t) = &saved.task {
                t.cancel();
            }
            s.stopping = true;
            s.preview = None;
        }
        let action = if apply {
            s.applying = true;
            s.notice = None;
            d.error = None;
            s.preview.clone().map(|review| Action::Workflow {
                review,
                selected: vec![],
            })
        } else {
            None
        };
        let owner = d.owner;
        if !close {
            self.dialog = Some(d);
        }
        if recheck {
            self.start_workflow_review();
        }
        if let (Some(action), Some(owner)) = (action, owner) {
            self.dispatch(Intent::Execute(owner, action));
            if self.write.is_none()
                && let Some(d) = &mut self.dialog
                && let Some(s) = &mut d.workflow
            {
                s.applying = false;
                d.error = Some(
                    "Could not start; wait for repository work to finish, then check again."
                        .into(),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_patch_reads_as_numbered_changes_with_raw_line_ids() {
        let patch = b"diff --git a/x b/x\n--- a/x\n+++ b/x\n@@ -3,3 +3,4 @@ fn connect()\n keep\n-old\n+new\n+more\n keep 2\n@@ -10 +11 @@\n-a\n+b\n\\ No newline at end of file\n";
        let found = changes(patch);
        assert_eq!(found.len(), 2);
        let first = &found[0];
        assert_eq!((first.first, first.last, first.context.as_str()), (3, 6, "fn connect()"));
        let rows: Vec<_> = first.rows.iter().map(|r| (r.id, r.old, r.new, r.sign)).collect();
        assert_eq!(rows, [(4, Some(3), Some(3), b' '), (5, Some(4), None, b'-'), (6, None, Some(4), b'+'), (7, None, Some(5), b'+'), (8, Some(5), Some(6), b' ')]);
        assert_eq!((found[1].first, found[1].last), (11, 11));
        assert_eq!(found[1].rows.iter().map(|r| r.id).collect::<Vec<_>>(), [10, 11]);
        assert_eq!(counts(patch), (3, 2));
    }

    #[test]
    fn choosing_lines_keeps_exact_ids_and_drops_an_empty_file() {
        let mut s = State::default();
        let all = [5, 6, 7, 11];
        choose_lines(&mut s.chosen, 0, &all, &[5, 6], true);
        assert_eq!(s.selection()[0].lines, [5, 6]);
        // A whole file narrowed to some lines keeps the rest.
        s.chosen.insert(1, None);
        choose_lines(&mut s.chosen, 1, &all, &[7], false);
        assert_eq!(s.selection()[1].lines, [5, 6, 11]);
        choose_lines(&mut s.chosen, 0, &all, &[5, 6], false);
        assert!(!s.chosen.contains_key(&0), "a file with no lines left is not chosen");
        assert!(chosen_line(&s.chosen, 1, 11) && !chosen_line(&s.chosen, 1, 7));
    }
}
