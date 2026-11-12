//! Pinned, cancellable stash inspection. Receivers are replaced on selection so
//! late results cannot overwrite a newer selection. Mutations use the existing bridge.
use crate::{
    app::*,
    backend::{Action, user_error},
    diff::{Kind, Patch},
    pages,
    style::*,
};
use eframe::egui::*;
use rocket_git::*;
use std::collections::HashMap;
fn preview_text(bytes: &[u8], limit: usize) -> String {
    display(crate::text::prefix(bytes, limit))
}
fn subject(entry: &StashEntry) -> String {
    let bytes = &entry.subject;
    let start = if bytes.starts_with(b"On ") || bytes.starts_with(b"WIP on ") {
        bytes
            .windows(2)
            .position(|p| p == b": ")
            .map_or(0, |i| i + 2)
    } else {
        0
    };
    preview_text(&bytes[start..], 180)
}

pub enum StashIntent {
    Select(StashEntry),
    Part(StashPart),
    Cancel,
    /// Review exporting a stash's tracked changes as a patch file.
    SavePatch(ObjectId),
    ApplySelected(StashEntry, StashPart),
}
#[derive(Default)]
pub struct State {
    pub selected: Option<StashEntry>,
    pub part: StashPart,
    pub patch: Option<Patch>,
    pub file: usize,
    pub error: Option<String>,
    pub notice: Option<String>,
    task: Option<ReadTask<Patch>>,
}
impl State {
    pub fn running(&self) -> bool {
        self.task.is_some()
    }
    pub fn stop(&mut self) {
        if let Some(task) = &self.task {
            task.cancel();
        }
    }
}
impl Rocket {
    pub(crate) fn stash_intent(&mut self, intent: StashIntent) {
        if let StashIntent::ApplySelected(entry, part) = &intent {
            self.open_stash_apply(entry.clone(), *part);
            return;
        }
        if let StashIntent::SavePatch(id) = &intent {
            // A stash commit against its first parent holds the saved tracked
            // changes, which is what the patch export reads for a commit.
            self.show_dialog(DialogKind::Patch(crate::patches::Mode::Export));
            if let Some(s) = self.dialog.as_mut().and_then(|d| d.patch.as_mut()) {
                s.source = 2;
                s.revision = id.to_string();
            }
            return;
        }
        let Some(backend) = self.backend.clone() else {
            return;
        };
        let Some(tab) = self.tabs.get_mut(self.active) else {
            return;
        };
        let Some(repo) = tab.snapshot.as_ref().map(|s| s.repository.clone()) else {
            return;
        };
        let state = &mut tab.stash;
        match intent {
            StashIntent::Cancel => {
                state.stop();
                return;
            }
            StashIntent::Select(entry) => {
                state.selected = Some(entry);
            }
            StashIntent::Part(part) => state.part = part,
            StashIntent::SavePatch(_) => return,
            StashIntent::ApplySelected(..) => return,
        }
        state.task = None;
        state.patch = None;
        state.file = 0;
        state.error = None;
        state.notice = None;
        let Some(entry) = state.selected.clone() else {
            return;
        };
        let part = state.part;
        match ReadTask::spawn(move |cancel| {
            backend.log.request("stash-preview", |log| {
                let diff = repo
                    .with_log(log.clone())
                    .stash_diff(&entry, part, &cancel)?;
                let patch = crate::file_history::historical_patch(&diff.patch);
                if cancel.is_cancelled() {
                    return Err(Error::Cancelled);
                }
                Ok(patch)
            })
        }) {
            Ok(task) => state.task = Some(task),
            Err(error) => state.error = Some(user_error(&error)),
        }
    }
    pub(crate) fn poll_stashes(&mut self, ctx: &Context) {
        for tab in &mut self.tabs {
            let state = &mut tab.stash;
            if let Some(result) = state.task.as_mut().and_then(ReadTask::try_finish) {
                state.task = None;
                match result {
                    Ok(patch) => state.patch = Some(patch),
                    Err(Error::Cancelled) => {
                        state.notice = Some(
                            crate::i18n::tr("Preview cancelled. Reselect the stash or a preview tab to retry.")
                                .into(),
                        )
                    }
                    Err(error) => state.error = Some(user_error(&error)),
                }
            }
            if state.running() {
                ctx.request_repaint_after(std::time::Duration::from_millis(30));
            }
        }
    }
}
fn register(ui: &Ui, widgets: &mut HashMap<String, UiControl>, id: String, r: &Response) {
    let rect = r.interact_rect.intersect(ui.clip_rect());
    if rect.is_positive() {
        widgets.insert(
            id,
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
    id: &str,
    label: &str,
    enabled: bool,
) -> bool {
    let r = ui.add_enabled(enabled, Button::new(label));
    register(ui, widgets, id.into(), &r);
    r.clicked()
}
fn list(
    ui: &mut Ui,
    entries: &[StashEntry],
    s: &State,
    intents: &mut Vec<Intent>,
    widgets: &mut HashMap<String, UiControl>,
) {
    ui.spacing_mut().item_spacing.y = 2.0;
    if entries.is_empty() {
        pages::empty(ui, crate::i18n::tr("No saved stashes."));
    }
    for entry in entries {
        let title = subject(entry);
        let detail = format!("stash@{{{}}} · {}", entry.index, &entry.id.as_str()[..8]);
        let r = pages::row(
            ui,
            pages::Row {
                image: Icon::Stash,
                title: &title,
                detail: &detail,
                pills: Vec::new(),
                indent: 0.0,
            },
            s.selected.as_ref() == Some(entry),
            true,
        )
        .on_hover_text(display(&entry.subject));
        register(ui, widgets, format!("stash-select-{}", entry.index), &r);
        if r.clicked() {
            intents.push(Intent::Stash(StashIntent::Select(entry.clone())));
            ui.close();
        }
    }
}
/// Stashes page. Wide: a list pane beside the pinned preview. Narrow: a picker above
/// it. Short: one title row that also carries the selection's actions.
pub(crate) fn view(
    ui: &mut Ui,
    tab: &mut Tab,
    writable: bool,
    intents: &mut Vec<Intent>,
    widgets: &mut HashMap<String, UiControl>,
) {
    let compact = ui.available_height() < 300.0;
    let entries = tab
        .snapshot
        .as_ref()
        .map_or(&[][..], |s| s.stashes.as_slice());
    let count = crate::i18n::trf!("Saved locally: {count}", count = entries.len());
    let running = tab.stash.running();
    let description = if compact {
        count
    } else {
        crate::i18n::trf!("{count} · newest first · applying keeps the stash", count)
    };
    // Short panes have no preview strip, so the title row also carries Apply and Drop.
    pages::page_title(ui, compact, Icon::Stash, crate::i18n::tr("Stashes"), &description, |ui| {
        save_and_cancel(ui, running, writable, intents, widgets);
        if compact {
            actions(ui, &tab.stash, entries, writable, intents, widgets);
        }
    });
    if ui.available_width() < 750.0 {
        Frame::new()
            .inner_margin(Margin::symmetric(12, 6))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                let r = ComboBox::from_id_salt(("stash-list", tab.id))
                    .width(ui.available_width())
                    .selected_text(tab.stash.selected.as_ref().map_or_else(
                        || crate::i18n::tr("Select a saved stash").into(),
                        |e| format!("stash@{{{}}} · {}", e.index, subject(e)),
                    ))
                    .show_ui(ui, |ui| {
                        // Rows are 46 points; the row pitch includes this spacing.
                        ui.spacing_mut().item_spacing.y = 2.0;
                        ScrollArea::vertical().show_rows(ui, 46.0, entries.len(), |ui, range| {
                            list(ui, &entries[range], &tab.stash, intents, widgets)
                        });
                    })
                    .response;
                register(ui, widgets, "stash-picker".into(), &r);
            });
        pages::divider(ui);
        inspect(
            ui,
            &mut tab.stash,
            entries,
            writable,
            intents,
            widgets,
            compact,
        );
    } else {
        pages::list_pane(
            ui,
            Id::unique(("stash-list", tab.id)),
            280.0,
            200.0..=380.0,
            |ui| {
                Frame::new()
                    .inner_margin(Margin::symmetric(8, 8))
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        ScrollArea::vertical()
                            .auto_shrink([false, false])
                            .show_rows(ui, 46.0, entries.len(), |ui, range| {
                                list(ui, &entries[range], &tab.stash, intents, widgets)
                            });
                    });
            },
        );
        CentralPanel::default()
            .frame(Frame::new().fill(BG()))
            .show(ui, |ui| {
                inspect(
                    ui,
                    &mut tab.stash,
                    entries,
                    writable,
                    intents,
                    widgets,
                    compact,
                )
            });
    }
}
/// Page actions, laid out right to left so the primary action sits outermost.
fn save_and_cancel(
    ui: &mut Ui,
    running: bool,
    writable: bool,
    intents: &mut Vec<Intent>,
    widgets: &mut HashMap<String, UiControl>,
) {
    let r = pages::primary(ui, writable, crate::i18n::tr("Save changes…"));
    register(ui, widgets, "stash-save".into(), &r);
    if r.clicked() {
        intents.push(Intent::Dialog(DialogKind::Stash));
    }
    let r=pages::ghost(ui,writable,crate::i18n::tr("Snapshot…")).on_hover_text(crate::i18n::tr("Save a stash while keeping working files and staging unchanged"));
    register(ui,widgets,"stash-snapshot".into(),&r);
    if r.clicked(){intents.push(Intent::KeepLocal{snapshot:true,paths:Vec::new()});}
    if running && button(ui, widgets, "stash-cancel", crate::i18n::tr("Stop"), true) {
        intents.push(Intent::Stash(StashIntent::Cancel));
    }
}
fn actions(
    ui: &mut Ui,
    s: &State,
    entries: &[StashEntry],
    writable: bool,
    intents: &mut Vec<Intent>,
    widgets: &mut HashMap<String, UiControl>,
) {
    let Some(entry) = &s.selected else {
        return;
    };
    let ready =
        writable && entries.iter().any(|e| e.id == entry.id) && s.patch.is_some() && !s.running();
    // Callers lay these out right to left, so Apply ends up outermost.
    let r = pages::primary(ui, ready, crate::i18n::tr("Apply…"));
    register(ui, widgets, "stash-apply".into(), &r);
    if r.clicked() {
        intents.push(Intent::Dialog(DialogKind::Confirm(Action::ApplyStash { entry: entry.clone(), restore_index: false }, crate::i18n::trf!("Apply stash {id} to the current working tree? The stash is kept.", id = entry.id))));
    }
    let r = pages::ghost(ui, ready, crate::i18n::tr("Apply selected…"));
    register(ui, widgets, "stash-apply-selected".into(), &r);
    if r.clicked() { intents.push(Intent::Stash(StashIntent::ApplySelected(entry.clone(), s.part))); }
    let r = pages::danger_ghost(ui, ready && entries.contains(entry), crate::i18n::tr("Drop…"));
    register(ui, widgets, "stash-drop".into(), &r);
    if r.clicked() {
        intents.push(Intent::Dialog(DialogKind::Confirm(Action::DropStash(entry.clone()), crate::i18n::trf!("Drop {stash} · {id}? Its saved changes may become unrecoverable.", stash = format!("stash@{{{}}}", entry.index), id = entry.id))));
    }
    // Exporting writes only the chosen patch file, never the repository.
    let r = pages::ghost(ui, entries.contains(entry) && !s.running(), crate::i18n::tr("Save as patch…"))
        .on_hover_text(crate::i18n::tr("Review exporting this stash's tracked changes (against the commit it was saved on) as a patch file. Untracked files saved in the stash are not included."));
    register(ui, widgets, "stash-save-patch".into(), &r);
    if r.clicked() {
        intents.push(Intent::Stash(StashIntent::SavePatch(entry.id.clone())));
    }
}
fn parts(
    ui: &mut Ui,
    s: &State,
    intents: &mut Vec<Intent>,
    widgets: &mut HashMap<String, UiControl>,
    compact: bool,
) {
    let choices = [
        (StashPart::Tracked, "stash-tracked", crate::i18n::tr("All tracked")),
        (StashPart::Index, "stash-index", crate::i18n::tr("Saved index")),
        (StashPart::Untracked, "stash-untracked", crate::i18n::tr("Untracked files")),
    ];
    if compact {
        let r = ComboBox::from_id_salt("stash-part")
            .width(145.0)
            .selected_text(choices.iter().find(|c| c.0 == s.part).unwrap().2)
            .show_ui(ui, |ui| {
                for (part, id, label) in choices {
                    let r = ui.selectable_label(s.part == part, label);
                    register(ui, widgets, id.into(), &r);
                    if r.clicked() {
                        intents.push(Intent::Stash(StashIntent::Part(part)));
                        ui.close();
                    }
                }
            })
            .response;
        register(ui, widgets, "stash-part-picker".into(), &r);
    } else {
        let items: Vec<_> = choices.iter().map(|c| (c.2, s.part == c.0)).collect();
        for ((part, id, _), r) in choices.into_iter().zip(pages::segments(ui, &items)) {
            register(ui, widgets, id.into(), &r);
            if r.clicked() {
                intents.push(Intent::Stash(StashIntent::Part(part)));
            }
        }
    }
}
fn files(ui: &mut Ui, s: &mut State, widgets: &mut HashMap<String, UiControl>) {
    let Some(patch) = s.patch.as_ref().filter(|p| !p.files.is_empty()) else {
        return;
    };
    s.file = s.file.min(patch.files.len() - 1);
    let r = ComboBox::from_id_salt("stash-files")
        .width(ui.available_width().min(420.0))
        .selected_text(&patch.files[s.file].label)
        .show_ui(ui, |ui| {
            for (i, file) in patch.files.iter().enumerate() {
                let r = ui.selectable_label(
                    s.file == i,
                    format!("{}  +{} −{}", file.label, file.added, file.removed),
                );
                register(ui, widgets, format!("stash-file-{i}"), &r);
                if r.clicked() {
                    s.file = i;
                }
            }
        })
        .response;
    register(ui, widgets, "stash-file-picker".into(), &r);
    if ui.available_width() > 120.0 {
        let file = &patch.files[s.file];
        ui.label(
            RichText::new(format!("+{}", file.added))
                .size(12.0)
                .color(GREEN()),
        );
        ui.label(
            RichText::new(format!("−{}", file.removed))
                .size(12.0)
                .color(RED()),
        );
    }
}
/// A status line: a tinted callout normally, one truncated line in short panes.
fn message(ui: &mut Ui, color: Color32, text: &str, compact: bool) {
    if compact {
        ui.add(Label::new(RichText::new(text).size(12.0).color(color)).truncate())
            .on_hover_text(text);
    } else {
        pages::callout(ui, color, text);
    }
}
fn inspect(
    ui: &mut Ui,
    s: &mut State,
    entries: &[StashEntry],
    writable: bool,
    intents: &mut Vec<Intent>,
    widgets: &mut HashMap<String, UiControl>,
    compact: bool,
) {
    let Some(entry) = s.selected.clone() else {
        Frame::new()
            .inner_margin(Margin::symmetric(24, 8))
            .show(ui, |ui| {
                pages::empty(ui, crate::i18n::tr("Select a stash to inspect its saved changes."))
            });
        return;
    };
    let listed = entries.iter().any(|e| e.id == entry.id);
    if !compact {
        pages::strip(ui, |ui| {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                actions(ui, s, entries, writable, intents, widgets);
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    ui.allocate_ui(vec2((ui.available_width() - 200.0).max(60.0), 28.0), |ui| {
                        ui.add(
                            Label::new(RichText::new(subject(&entry)).font(semibold(13.5)))
                                .truncate(),
                        )
                        .on_hover_text(format!(
                            "{}\n{}",
                            entry.id,
                            display(&entry.subject)
                        ));
                    });
                    pages::pill(ui, &format!("stash@{{{}}}", entry.index), pages::INFO());
                    ui.label(
                        RichText::new(&entry.id.as_str()[..12])
                            .monospace()
                            .size(12.0)
                            .color(FAINT()),
                    );
                });
            });
        });
    }
    let shown = Frame::new()
        .inner_margin(Margin::symmetric(12, if compact { 4 } else { 8 }))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 6.0;
            ui.horizontal_wrapped(|ui| {
                parts(ui, s, intents, widgets, compact);
                files(ui, s, widgets);
            });
            if !listed {
                message(
                    ui,
                    AMBER(),
                    crate::i18n::tr("This stash is no longer in the list. The preview remains pinned to its saved commit."),
                    compact,
                );
            } else if !entries.contains(&entry) {
                message(
                    ui,
                    AMBER(),
                    crate::i18n::tr("Stash positions changed. Reselect this entry before dropping it."),
                    compact,
                );
            }
            if s.running() {
                // Centered while nothing is shown; beside a patch that stays on screen.
                if s.patch.is_none() {
                    pages::loading(ui, crate::i18n::tr("Reading saved changes…"));
                } else {
                    pages::busy_line(ui, crate::i18n::tr("Reading saved changes…"));
                }
            }
            if let Some(error) = &s.error {
                message(ui, RED(), error, compact);
            }
            if let Some(notice) = &s.notice {
                pages::note(ui, notice);
            }
            let patch = s.patch.as_ref()?;
            if patch.limited {
                message(
                    ui,
                    AMBER(),
                    crate::i18n::tr("Preview limit reached; this is not the complete patch."),
                    compact,
                );
            }
            if patch.files.is_empty() {
                pages::note(ui, crate::i18n::tr("No saved changes in this section."));
                return None;
            }
            Some(())
        })
        .inner;
    if shown.is_none() {
        return;
    }
    let Some(file) = s.patch.as_ref().map(|p| &p.files[s.file]) else {
        return;
    };
    pages::divider(ui);
    // Each row is exactly 21 points so the virtualized row range stays accurate.
    ui.spacing_mut().item_spacing.y = 0.0;
    ui.spacing_mut().interact_size.y = 21.0;
    let out = ScrollArea::both()
        .id_salt(("stash-diff", &entry.id, s.part as u8, s.file))
        .auto_shrink([false, false])
        .min_scrolled_height(0.0)
        .show_rows(ui, 21.0, file.lines.len(), |ui, range| {
            for i in range {
                let line = &file.lines[i];
                let (color, tint) = match line.kind {
                    Kind::Added => (GREEN(), Some(GREEN())),
                    Kind::Removed => (RED(), Some(RED())),
                    Kind::Hunk => (BLUE(), Some(BLUE())),
                    Kind::Header => (MUTED(), None),
                    Kind::Context => (TEXT(), None),
                };
                let top = ui.cursor().top();
                if let Some(tint) = tint {
                    ui.painter().rect_filled(
                        Rect::from_x_y_ranges(ui.clip_rect().x_range(), top..=top + 21.0),
                        0.0,
                        tint.gamma_multiply(0.08),
                    );
                }
                ui.horizontal(|ui| {
                    ui.set_min_height(21.0);
                    ui.add_space(12.0);
                    ui.add(Label::new(RichText::new(&line.text).monospace().color(color)).extend());
                });
            }
        });
    let rect = out.inner_rect.intersect(ui.clip_rect());
    if rect.is_positive() {
        widgets.insert(
            "stash-diff-scroll".into(),
            UiControl {
                rect,
                enabled: true,
            },
        );
    }
}
