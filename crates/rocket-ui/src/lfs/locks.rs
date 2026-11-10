use super::*;
#[derive(Default)]
pub struct DialogState {
    pub list: Option<LfsLocks>,
    pub selected: Option<String>,
    pub path: String,
    pub action: Option<LfsLockAction>,
    pub preview: Option<Arc<LfsLockPreview>>,
    pub force_confirmed: bool,
    pub list_task: Option<ReadTask<LfsLocks>>,
    pub task: Option<ReadTask<LfsLockPreview>>,
}
impl DialogState {
    pub fn running(&self) -> bool {
        self.list_task.is_some() || self.task.is_some()
    }
    pub fn selected_entry(&self) -> Option<&LfsLock> {
        self.list
            .as_ref()?
            .entries
            .iter()
            .find(|l| Some(&l.id) == self.selected.as_ref())
    }
}
impl Rocket {
    pub(crate) fn start_lfs_locks(&mut self) {
        let Some(d) = &mut self.dialog else { return };
        let Some(s) = &mut d.lfs else { return };
        if !s.locks_tab || s.applying {
            return;
        }
        // A fresh review/refresh owns its message; an earlier operation must not
        // look like the result of this request. Its full outcome stays in Activity.
        s.notice = None;
        let locks = &mut s.locks;
        locks.task = None;
        locks.list_task = None;
        locks.preview = None;
        locks.force_confirmed = false;
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
        let scope = self.lfs_lock_source_scope.clone();
        let remote = s.remote.clone();
        if remote.is_empty() {
            d.error = Some(crate::i18n::tr("Add a remote before reading LFS locks.").into());
            return;
        }
        if let Some(action) = locks.action.clone() {
            match ReadTask::spawn(move |cancel| {
                backend.lfs_lock_preview(&path, &remote, action, scope.as_ref(), &cancel)
            }) {
                Ok(t) => locks.task = Some(t),
                Err(e) => d.error = Some(user_error(&e)),
            }
        } else {
            locks.list = None;
            match ReadTask::spawn(move |cancel| {
                backend.lfs_locks(&path, &remote, scope.as_ref(), &cancel)
            }) {
                Ok(t) => locks.list_task = Some(t),
                Err(e) => d.error = Some(user_error(&e)),
            }
        }
    }
    pub(crate) fn poll_lfs_locks(&mut self) {
        let Some(d) = &mut self.dialog else { return };
        let Some(s) = &mut d.lfs else { return };
        let l = &mut s.locks;
        if let Some(result) = l.list_task.as_mut().and_then(ReadTask::try_finish) {
            l.list_task = None;
            match result {
                Ok(list) => {
                    if !list
                        .entries
                        .iter()
                        .any(|e| Some(&e.id) == l.selected.as_ref())
                    {
                        l.selected = None;
                    }
                    l.list = Some(list);
                }
                Err(e) => {
                    l.list = None;
                    l.selected = None;
                    d.error = Some(user_error(&e));
                }
            }
        }
        if let Some(result) = l.task.as_mut().and_then(ReadTask::try_finish) {
            l.task = None;
            match result {
                Ok(p) => l.preview = Some(Arc::new(p)),
                Err(e) => d.error = Some(user_error(&e)),
            }
        }
    }
    pub(crate) fn finish_lfs_locks(&mut self, owner: Option<u64>, outcome: &Outcome) {
        let Some(d) = &mut self.dialog else { return };
        let Some(s) = &mut d.lfs else { return };
        if d.owner != owner || !s.applying {
            return;
        }
        s.applying = false;
        s.locks.preview = None;
        s.locks.action = None;
        s.locks.force_confirmed = false;
        s.locks.list = outcome.lfs_locks.clone();
        s.locks.selected = None;
        s.notice = Some(format!(
            "{}\n{}",
            outcome.summary,
            outcome.details.join("\n")
        ));
    }
    /// Lock, unlock or force-unlock review over the Git LFS page. The caller has
    /// already checked that the page belongs to the active repository.
    pub(crate) fn lfs_lock_review(&mut self, ctx: &Context) {
        let writable = self.can_write();
        let Some(mut d) = self.dialog.take() else {
            return;
        };
        let s = d.lfs.get_or_insert_with(Default::default);
        let Some(action) = s.locks.action.clone() else {
            self.dialog = Some(d);
            return;
        };
        let waiting = s.locks.running() || s.applying;
        let mut f = Flags::default();
        let force = action.force();
        let (title, subtitle) = match &action {
            LfsLockAction::Lock(_) => (
                crate::i18n::tr("Lock this file"),
                crate::i18n::tr("Other accounts see the lock until you unlock it."),
            ),
            LfsLockAction::Unlock { force: true, .. } => (
                crate::i18n::tr("Force unlock this lock?"),
                crate::i18n::tr("This removes a lock that belongs to another account."),
            ),
            LfsLockAction::Unlock { .. } => {
                (crate::i18n::tr("Unlock your file"), crate::i18n::tr("Releases your lock on the server."))
            }
        };
        let head = pages::ReviewHeader {
            image: if matches!(action, LfsLockAction::Lock(_)) {
                Icon::Lock
            } else {
                Icon::Database
            },
            title,
            subtitle,
            danger: force,
        };
        let running = pages::Running::of(self.write.as_ref());
        let response = pages::review(
            ctx,
            "lfs-locks-dialog",
            head,
            &mut self.frame_widgets,
            "dialog-lfs-scroll",
            !s.applying,
            |ui, part, widgets| match part {
                pages::Part::Body => {
                    let l = &mut s.locks;
                    if let Some(p) = &l.preview {
                        pages::caption(ui, crate::i18n::tr("What will happen"));
                        pages::field(ui, crate::i18n::tr("File"), p.action().path(), true);
                        pages::field(
                            ui,
                            crate::i18n::tr("Server"),
                            &format!("{} · {}", p.remote(), p.source()),
                            true,
                        );
                        if let LfsLockAction::Unlock { lock, .. } = p.action() {
                            pages::field(
                                ui,
                                crate::i18n::tr("Lock owner"),
                                if lock.owner.is_empty() {
                                    crate::i18n::tr("Unavailable")
                                } else {
                                    &lock.owner
                                },
                                false,
                            );
                            pages::field(
                                ui,
                                crate::i18n::tr("Lock ID"),
                                &format!("{} · {}", lock.id, lock.locked_at),
                                true,
                            );
                        }
                        pages::field(
                            ui,
                            crate::i18n::tr("Your files"),
                            crate::i18n::tr("Contents, staging and commits stay as they are"),
                            false,
                        );
                        if p.makes_writable() {
                            pages::callout(
                                ui,
                                AMBER(),
                                crate::i18n::tr("Git LFS will make this selected read-only file writable after locking."),
                            );
                        }
                        if p.action().force() {
                            pages::callout(ui, AMBER(), crate::i18n::tr("This removes another account's lock."));
                            let r = ui.add_enabled(
                                !waiting,
                                checkbox(
                                    &mut l.force_confirmed,
                                    crate::i18n::tr("I understand this removes another account's lock."),
                                ),
                            );
                            control(widgets, "dialog-lfs-lock-force-confirm", &r, ui.clip_rect());
                        }
                    }
                    if let Some(notice) = &s.notice {
                        pages::callout(ui, BLUE(), notice);
                    }
                    // A running operation shows its status and Stop in the footer.
                    if waiting && !s.applying {
                        pages::waiting(ui, crate::i18n::tr("Reading server locks and verifying ownership…"));
                    }
                    if let Some(error) = &d.error {
                        pages::callout(ui, RED(), error);
                    }
                }
                pages::Part::Footer if s.applying => {
                    f.cancel = pages::running_footer(ui, widgets, &running, &[]);
                }
                pages::Part::Footer => {
                    let l = &s.locks;
                    let ready = l
                        .preview
                        .as_ref()
                        .is_some_and(|p| !p.action().force() || l.force_confirmed);
                    let label = match l.action {
                        Some(LfsLockAction::Lock(_)) => crate::i18n::tr("Lock file"),
                        Some(LfsLockAction::Unlock { force: true, .. }) => crate::i18n::tr("Force unlock"),
                        _ => crate::i18n::tr("Unlock file"),
                    };
                    let enabled = !waiting && writable && ready && d.error.is_none();
                    let r = if force {
                        pages::danger(ui, enabled, label)
                    } else {
                        pages::primary(ui, enabled, label)
                    };
                    control(widgets, "dialog-submit", &r, ui.clip_rect());
                    f.submit = r.clicked();
                    let r = ui.add_enabled(!waiting, Button::new(crate::i18n::tr("Review again")));
                    control(widgets, "dialog-preview-refresh", &r, ui.clip_rect());
                    f.refresh |= r.clicked();
                }
            },
        );
        // The × returns to the Locks tab; automation's old Cancel id is that ×.
        if let Some(close) = self.frame_widgets.get("dialog-close").copied() {
            self.frame_widgets.insert("dialog-lfs-back".into(), close);
        }
        f.back |= response.should_close() && !s.applying;
        self.finish_lfs_frame(d, f);
    }
}
