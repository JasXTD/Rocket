//! Whole-file previews, recovery selection and cancellable background reads.
use crate::{
    app::*,
    backend::{Action, Outcome, user_error},
    pages,
    style::*,
};
use eframe::egui::*;
use rocket_git::*;
use std::sync::Arc;
#[derive(Default)]
pub struct DialogState {
    pub path: Option<RepoPath>,
    pub source: Option<FileEditSource>,
    pub revision: String,
    pub preview: Option<Arc<FileEditPreview>>,
    pub task: Option<ReadTask<FileEditPreview>>,
    pub backups: Vec<FileBackup>,
    pub list_task: Option<ReadTask<Vec<FileBackup>>>,
    pub applying: bool,
    pub notice: Option<String>,
}
impl DialogState {
    pub fn running(&self) -> bool {
        self.task.is_some() || self.list_task.is_some()
    }
}
impl Rocket {
    pub(crate) fn start_file_edit(&mut self) {
        let Some(d) = &mut self.dialog else { return };
        let DialogKind::FileEdit(initial) = &d.kind else {
            return;
        };
        let s = d.file_edit.get_or_insert_with(|| DialogState {
            path: initial.as_ref().map(|i| i.0.clone()),
            source: initial.as_ref().map(|i| i.1.clone()),
            revision: initial
                .as_ref()
                .and_then(|i| {
                    if let FileEditSource::Commit(r) = &i.1 {
                        Some(r.clone())
                    } else {
                        None
                    }
                })
                .unwrap_or_else(|| "HEAD".into()),
            ..Default::default()
        });
        if s.applying {
            return;
        }
        d.error = None;
        s.notice = None;
        s.task = None;
        s.list_task = None;
        s.preview = None;
        let Some(backend) = self.backend.clone() else {
            return;
        };
        let Some(root) = self
            .tabs
            .iter()
            .find(|t| Some(t.id) == d.owner)
            .map(|t| t.path.clone())
        else {
            return;
        };
        if let (Some(path), Some(mut source)) = (s.path.clone(), s.source.clone()) {
            if matches!(source, FileEditSource::Commit(_)) {
                source = FileEditSource::Commit(s.revision.clone());
                s.source = Some(source.clone())
            }
            match ReadTask::spawn(move |cancel| {
                backend.file_edit_preview(&root, &path, source, &cancel)
            }) {
                Ok(task) => s.task = Some(task),
                Err(e) => d.error = Some(user_error(&e)),
            }
        } else {
            match ReadTask::spawn(move |cancel| backend.file_backups(&root, &cancel)) {
                Ok(task) => s.list_task = Some(task),
                Err(e) => d.error = Some(user_error(&e)),
            }
        }
    }
    pub(crate) fn poll_file_edit(&mut self) {
        let Some(d) = &mut self.dialog else { return };
        let Some(s) = &mut d.file_edit else { return };
        if let Some(result) = s.task.as_mut().and_then(ReadTask::try_finish) {
            s.task = None;
            match result {
                Ok(p) => s.preview = Some(Arc::new(p)),
                Err(e) => d.error = Some(user_error(&e)),
            }
        }
        if let Some(result) = s.list_task.as_mut().and_then(ReadTask::try_finish) {
            s.list_task = None;
            match result {
                Ok(v) => s.backups = v,
                Err(e) => d.error = Some(user_error(&e)),
            }
        }
    }
    pub(crate) fn finish_file_edit(&mut self, owner: Option<u64>, outcome: &Outcome) {
        let Some(d) = &mut self.dialog else { return };
        if d.owner != owner {
            return;
        }
        let Some(s) = &mut d.file_edit else { return };
        if !s.applying {
            return;
        }
        s.applying = false;
        s.preview = None;
        let text = format!("{}\n{}", outcome.summary, outcome.details.join("\n"));
        if outcome.command_succeeded && !outcome.recovery_pending {
            s.notice = Some(text);
            d.error = None
        } else {
            d.error = Some(text)
        }
    }
    pub(crate) fn file_edit_dialog(&mut self, ctx: &Context) {
        let can_write = self.can_write();
        let running = pages::Running::of(self.write.as_ref());
        let Some(mut d) = self.dialog.take() else {
            return;
        };
        let Some(s) = &mut d.file_edit else {
            self.dialog = Some(d);
            return;
        };
        let (mut refresh, mut close, mut apply, mut cancel) = (false, false, false, false);
        let mut backup = None;
        let list = s.path.is_none();
        let discard = !list && matches!(s.source, Some(FileEditSource::Index));
        let head = pages::ReviewHeader {
            image: Icon::File,
            title: if list {
                "File backups"
            } else {
                match s.source {
                    Some(FileEditSource::Index) => "Discard unstaged changes",
                    Some(FileEditSource::Backup(_)) => "Restore recovery copy",
                    _ => "Restore file from commit",
                }
            },
            subtitle: "Only the working file changes; a recovery copy is kept.",
            danger: discard,
        };
        let response = pages::wide_review(
            ctx,
            "file-edit-dialog",
            head,
            &mut self.frame_widgets,
            "dialog-file-scroll",
            true,
            |ui, part, widgets| match part {
                pages::Part::Body => {
                    // Results lead the body so a long preview never hides them.
                    if let Some(notice) = &s.notice {
                        pages::callout(ui, GREEN(), notice);
                    }
                    if let Some(error) = &d.error {
                        pages::callout(ui, AMBER(), error);
                    }
                    if list {
                        pages::caption(ui, "Restore a file from a commit");
                        ui.label("Repository-relative path");
                        let r = ui.add_enabled(
                            !s.running(),
                            TextEdit::singleline(&mut d.first)
                                .hint_text("src/file.rs")
                                .desired_width(f32::INFINITY),
                        );
                        register(ui, widgets, "dialog-file-path", &r);
                        ui.label("Commit, branch or tag");
                        let r = ui.add_enabled(
                            !s.running(),
                            TextEdit::singleline(&mut s.revision)
                                .hint_text("Commit, branch or tag")
                                .desired_width(f32::INFINITY),
                        );
                        register(ui, widgets, "dialog-file-revision", &r);
                        let r = ui.add_enabled(
                            !s.running() && !d.first.is_empty(),
                            Button::new("Review restore from commit"),
                        );
                        register(ui, widgets, "dialog-file-review", &r);
                        if r.clicked() {
                            match RepoPath::new(d.first.as_bytes().to_vec()) {
                                Ok(path) => {
                                    s.path = Some(path);
                                    s.source = Some(FileEditSource::Commit(s.revision.clone()));
                                    refresh = true
                                }
                                Err(e) => d.error = Some(user_error(&e)),
                            }
                        }
                        ui.add_space(6.0);
                        pages::caption(ui, "Recovery copies · newest first");
                        if s.backups.is_empty() && !s.running() {
                            pages::note(
                                ui,
                                "No completed recovery copies in this working folder yet.",
                            );
                        }
                        let rows = ui
                            .scope(|ui| {
                                ui.spacing_mut().item_spacing.y = 2.0;
                                ScrollArea::vertical()
                                    .id_salt("file-backup-rows")
                                    .max_height(225.0)
                                    .auto_shrink([false, true])
                                    .show_rows(ui, 46.0, s.backups.len(), |ui, range| {
                                        for i in range {
                                            let b = &s.backups[i];
                                            let path = b.path().display_lossy().into_owned();
                                            let detail = format!(
                                                "{} · {}",
                                                if b.original_present() {
                                                    "Restore saved contents"
                                                } else {
                                                    "Return to absent file"
                                                },
                                                b.id()
                                            );
                                            let r = pages::row(
                                                ui,
                                                pages::Row {
                                                    image: Icon::File,
                                                    title: &path,
                                                    detail: &detail,
                                                    pills: vec![],
                                                    indent: 0.0,
                                                },
                                                false,
                                                !s.running(),
                                            );
                                            register(
                                                ui,
                                                widgets,
                                                format!("dialog-file-backup-{i}"),
                                                &r,
                                            );
                                            if r.clicked() {
                                                backup = Some(b.clone())
                                            }
                                            r.on_hover_text(path);
                                        }
                                    })
                            })
                            .inner;
                        let rect = rows.inner_rect.intersect(ui.clip_rect());
                        if rect.is_positive() {
                            widgets.insert(
                                "dialog-file-backups-scroll".into(),
                                UiControl {
                                    rect,
                                    enabled: true,
                                },
                            );
                        }
                    } else {
                        pages::caption(ui, "What will happen");
                        pages::well(ui, |ui| {
                            pages::field(
                                ui,
                                "Working file",
                                &s.path.as_ref().unwrap().display_lossy(),
                                true,
                            );
                            if let Some(p) = &s.preview {
                                if let Some(id) = p.commit() {
                                    pages::field(ui, "Pinned commit", &id.to_string(), true);
                                }
                                pages::field(ui, "Result", p.conversion(), false);
                            }
                        });
                        if matches!(s.source, Some(FileEditSource::Commit(_))) {
                            ui.label("Source revision");
                            let r = ui.add_enabled(
                                !s.running() && !s.applying,
                                TextEdit::singleline(&mut s.revision).desired_width(f32::INFINITY),
                            );
                            register(ui, widgets, "dialog-file-revision", &r);
                            if r.changed() {
                                s.preview = None;
                                s.notice = None;
                            }
                        }
                        if let Some(p) = &s.preview {
                            if ui.available_width() > 570.0 {
                                ui.columns(2, |cols| {
                                    preview(&mut cols[0], "Current working file", p.before());
                                    preview(&mut cols[1], "Reviewed result", p.after());
                                });
                            } else {
                                preview(ui, "Current working file", p.before());
                                preview(ui, "Reviewed result", p.after());
                            }
                            if p.after().is_none() {
                                pages::callout(
                                    ui,
                                    AMBER(),
                                    "The file will leave the working folder. Its contents will remain in a recovery copy.",
                                );
                            }
                        }
                    }
                    if s.running() {
                        pages::waiting(ui, "Reading file, source and repository state…");
                    }
                }
                pages::Part::Footer if s.applying => {
                    cancel = pages::running_footer(ui, widgets, &running, &["dialog-cancel"]);
                }
                pages::Part::Footer => {
                    if !list {
                        let ready = can_write && !s.running() && !s.applying && s.preview.is_some();
                        let r = if discard {
                            pages::danger(ui, ready, "Create backup & apply")
                        } else {
                            pages::primary(ui, ready, "Create backup & apply")
                        };
                        register(ui, widgets, "dialog-submit", &r);
                        apply = r.clicked();
                    }
                    let r = ui.add_enabled(
                        !s.running() && !s.applying,
                        Button::new(if list {
                            "Refresh backups"
                        } else {
                            "Review again"
                        }),
                    );
                    register(ui, widgets, "dialog-preview-refresh", &r);
                    refresh |= r.clicked();
                    if !list {
                        let r = ui
                            .add_enabled(!s.running() && !s.applying, Button::new("File backups"));
                        register(ui, widgets, "dialog-file-list", &r);
                        if r.clicked() {
                            s.path = None;
                            s.source = None;
                            s.preview = None;
                            refresh = true;
                        }
                    }
                    // Stop the running read; the dialog stays open.
                    if s.running() {
                        let r = ui.button("Stop");
                        register(ui, widgets, "dialog-cancel", &r);
                        cancel = r.clicked();
                    }
                }
            },
        );
        // The ×, Esc or a click outside closes only an idle dialog.
        close |= response.should_close() && !s.applying && !s.running();
        if let Some(b) = backup {
            s.path = Some(b.path().clone());
            s.source = Some(FileEditSource::Backup(b));
            refresh = true;
        }
        let action = if apply {
            s.preview.clone().map(Action::FileEdit)
        } else {
            None
        };
        if cancel && !s.applying {
            s.task = None;
            s.list_task = None;
            s.preview = None;
            s.notice = Some("Preview cancelled. No working file was changed.".into());
        }
        self.dialog = if close { None } else { Some(d) };
        if cancel && self.write.is_some() {
            self.cancel_write();
        }
        if refresh {
            self.start_file_edit();
        }
        if let Some(action) = action
            && let Some(owner) = self.dialog.as_ref().and_then(|d| d.owner)
        {
            self.execute(owner, action);
            if self.write.is_some()
                && let Some(s) = self.dialog.as_mut().and_then(|d| d.file_edit.as_mut())
            {
                s.applying = true;
            }
        }
    }
}
fn preview(ui: &mut Ui, title: &str, bytes: Option<&[u8]>) {
    pages::caption(ui, title);
    let Some(bytes) = bytes else {
        ui.label(RichText::new("File absent").color(MUTED()));
        return;
    };
    pages::note(ui, &format!("{} bytes", bytes.len()));
    pages::well(ui, |ui| {
        if bytes.contains(&0) || std::str::from_utf8(bytes).is_err() {
            ui.label("Binary content · exact bytes will be used");
        } else if bytes.is_empty() {
            ui.label("Empty file");
        } else {
            let text = std::str::from_utf8(bytes).unwrap();
            ui.add(
                Label::new(RichText::new(text.chars().take(1800).collect::<String>()).monospace())
                    .wrap(),
            );
            if text.chars().take(1801).count() > 1800 {
                pages::callout(
                    ui,
                    AMBER(),
                    "Showing the first 1,800 characters; the whole file is used.",
                );
            }
        }
    });
}

fn register(
    ui: &Ui,
    widgets: &mut std::collections::HashMap<String, UiControl>,
    key: impl Into<String>,
    r: &Response,
) {
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
