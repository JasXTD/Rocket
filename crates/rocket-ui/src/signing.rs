//! Explicit signature reads; no signer runs during repository refresh or drawing.
use crate::{
    app::*,
    backend::{Action, Outcome, user_error},
    pages::{self, control},
    style::*,
};
use eframe::egui::*;
use rocket_git::*;
#[derive(Default)]
pub struct DialogState {
    pub settings: Option<RepositorySigning>,
    pub options: SigningOptions,
    pub key: String,
    pub allowed: String,
    pub settings_task: Option<ReadTask<RepositorySigning>>,
    pub verify_task: Option<ReadTask<SignatureVerification>>,
    pub verification: Option<SignatureVerification>,
    pub reviewed: bool,
    pub applying: bool,
    pub notice: Option<String>,
}
impl DialogState {
    pub fn running(&self) -> bool {
        self.settings_task.is_some() || self.verify_task.is_some()
    }
}
impl Rocket {
    pub(crate) fn start_signing_read(&mut self) {
        let Some(d) = &mut self.dialog else { return };
        let DialogKind::Signing(target) = &d.kind else {
            return;
        };
        let s = d.signing.get_or_insert_with(Default::default);
        if s.applying {
            return;
        }
        s.settings_task = None;
        s.verify_task = None;
        s.verification = None;
        s.reviewed = false;
        d.error = None;
        s.notice = None;
        let (Some(backend), Some(path)) = (
            self.backend.clone(),
            self.tabs
                .iter()
                .find(|t| Some(t.id) == d.owner)
                .map(|t| t.path.clone()),
        ) else {
            return;
        };
        if let Some(target) = target.clone() {
            match ReadTask::spawn(move |cancel| backend.verify_signature(&path, &target, &cancel)) {
                Ok(task) => s.verify_task = Some(task),
                Err(e) => d.error = Some(user_error(&e)),
            }
        } else {
            s.settings = None;
            match ReadTask::spawn(move |cancel| backend.signing(&path, &cancel)) {
                Ok(task) => s.settings_task = Some(task),
                Err(e) => d.error = Some(user_error(&e)),
            }
        }
    }
    pub(crate) fn poll_signing_read(&mut self) {
        let Some(d) = &mut self.dialog else { return };
        let Some(s) = &mut d.signing else { return };
        if let Some(result) = s.settings_task.as_mut().and_then(ReadTask::try_finish) {
            s.settings_task = None;
            match result {
                Ok(settings) => {
                    s.options = settings.local.clone();
                    s.key = s.options.key.clone().unwrap_or_default();
                    s.allowed = s.options.allowed_signers.clone().unwrap_or_default();
                    s.settings = Some(settings)
                }
                Err(e) => d.error = Some(user_error(&e)),
            }
        }
        if let Some(result) = s.verify_task.as_mut().and_then(ReadTask::try_finish) {
            s.verify_task = None;
            match result {
                Ok(v) => s.verification = Some(v),
                Err(e) => d.error = Some(user_error(&e)),
            }
        }
    }
    pub(crate) fn finish_signing_dialog(&mut self, owner: Option<u64>, outcome: &Outcome) {
        let Some(d) = &mut self.dialog else { return };
        if d.owner != owner {
            return;
        }
        let Some(s) = &mut d.signing else { return };
        if !s.applying {
            return;
        }
        s.applying = false;
        s.reviewed = false;
        s.settings = None;
        s.notice = Some(outcome.summary.clone());
        if !outcome.command_succeeded || !outcome.details.is_empty() {
            d.error = Some(outcome.details.join("\n"));
        }
    }
    pub(crate) fn signing_dialog(&mut self, ctx: &Context) {
        let writable = self.can_write();
        let Some(mut d) = self.dialog.take() else {
            return;
        };
        let DialogKind::Signing(target) = d.kind.clone() else {
            self.dialog = Some(d);
            return;
        };
        let s = d.signing.get_or_insert_with(Default::default);
        if d.owner != self.tab().map(|t| t.id) && !s.applying {
            return;
        }
        let busy = s.running() || s.applying;
        let running = pages::Running::of(self.write.as_ref());
        let mut refresh = false;
        let mut close = false;
        let mut cancel = false;
        let mut save = false;
        let head = pages::ReviewHeader {
            image: Icon::Tag,
            title: if target.is_some() {
                "Signature details"
            } else {
                "Repository signing"
            },
            subtitle: if target.is_some() {
                "Check one exact object with Git and your configured trust policy."
            } else {
                "Local overrides for this repository. Blank values use inherited settings."
            },
            danger: false,
        };
        let response = pages::review(
            ctx,
            "rocket-signing",
            head,
            &mut self.frame_widgets,
            "dialog-signing-scroll",
            !s.applying,
            |ui, part, widgets| match part {
                pages::Part::Body => {
                    if let Some(target) = &target {
                        pages::field(ui, "Object", target.id().as_str(), true);
                        pages::field(ui, "Kind", target.kind(), false);
                        if let Some(v) = &s.verification {
                            pages::callout(
                                ui,
                                match v.status {
                                    SignatureStatus::Valid => BLUE(),
                                    SignatureStatus::Invalid | SignatureStatus::Revoked => RED(),
                                    _ => AMBER(),
                                },
                                v.status.label(),
                            );
                            pages::field(ui, "Format", v.format, false);
                            if let Some(signer) = &v.signer {
                                pages::field(ui, "Signer", signer, false);
                            }
                            if let Some(fingerprint) = &v.fingerprint {
                                pages::field(ui, "Fingerprint", fingerprint, true);
                            }
                            pages::field(ui, "Trust", &v.trust, false);
                            pages::note(ui, v.status.guidance());
                        }
                    } else if let Some(settings) = &s.settings {
                        if s.reviewed {
                            pages::caption(ui, "Review local overrides");
                            pages::field(
                                ui,
                                "Format",
                                s.options.format.as_deref().unwrap_or("Inherited"),
                                false,
                            );
                            pages::field(
                                ui,
                                "Signing key",
                                s.options.key.as_deref().unwrap_or("Inherited"),
                                false,
                            );
                            pages::field(
                                ui,
                                "SSH trust file",
                                s.options.allowed_signers.as_deref().unwrap_or("Inherited"),
                                false,
                            );
                            pages::field(
                                ui,
                                "Sign commits",
                                match s.options.sign_commits {
                                    Some(true) => "Enabled",
                                    Some(false) => "Disabled",
                                    None => "Inherited",
                                },
                                false,
                            );
                        } else {
                            ui.label("Signing format");
                            ui.horizontal_wrapped(|ui| {
                                for (value, label, id) in [
                                    (None, "Inherited", "dialog-sign-format-inherit"),
                                    (Some("ssh"), "SSH", "dialog-sign-format-ssh"),
                                    (Some("openpgp"), "OpenPGP", "dialog-sign-format-gpg"),
                                ] {
                                    let r = ui.selectable_label(
                                        s.options.format.as_deref() == value,
                                        label,
                                    );
                                    control(widgets, id, &r, ui.clip_rect());
                                    if r.clicked() {
                                        s.options.format = value.map(str::to_owned);
                                    }
                                }
                            });
                            ui.label("Signing key · existing file, public key or fingerprint");
                            let r = ui
                                .add(TextEdit::singleline(&mut s.key).desired_width(f32::INFINITY));
                            control(widgets, "dialog-sign-key", &r, ui.clip_rect());
                            ui.label("SSH allowed signers file");
                            let r = ui.add(
                                TextEdit::singleline(&mut s.allowed).desired_width(f32::INFINITY),
                            );
                            control(widgets, "dialog-sign-allowed", &r, ui.clip_rect());
                            ui.label("Sign new commits");
                            ui.horizontal_wrapped(|ui| {
                                for (value, label, id) in [
                                    (None, "Inherited", "dialog-sign-commit-inherit"),
                                    (Some(true), "Enabled", "dialog-sign-commit-on"),
                                    (Some(false), "Disabled", "dialog-sign-commit-off"),
                                ] {
                                    let r =
                                        ui.selectable_label(s.options.sign_commits == value, label);
                                    control(widgets, id, &r, ui.clip_rect());
                                    if r.clicked() {
                                        s.options.sign_commits = value;
                                    }
                                }
                            });
                            s.options.key = (!s.key.is_empty()).then(|| s.key.clone());
                            s.options.allowed_signers =
                                (!s.allowed.is_empty()).then(|| s.allowed.clone());
                            pages::caption(ui, "Effective settings · before saving");
                            pages::field(ui, "Format", settings.format(), false);
                            pages::field(ui, "Signer program", settings.program(), false);
                            pages::field(
                                ui,
                                "SSH trust file",
                                settings
                                    .effective
                                    .allowed_signers
                                    .as_deref()
                                    .unwrap_or("Not configured"),
                                false,
                            );
                            pages::field(
                                ui,
                                "Key",
                                settings.effective.key.as_deref().unwrap_or("Git default"),
                                false,
                            );
                            pages::field(
                                ui,
                                "Sign commits",
                                if settings.effective.sign_commits.unwrap_or(false) {
                                    "Enabled"
                                } else {
                                    "Disabled"
                                },
                                false,
                            );
                        }
                    }
                    // Saving shows its status and Stop in the footer.
                    if busy && !s.applying {
                        pages::waiting(
                            ui,
                            if target.is_some() {
                                "Verifying with Git…"
                            } else {
                                "Reading signing settings…"
                            },
                        );
                    }
                    if let Some(notice) = &s.notice {
                        pages::callout(ui, BLUE(), notice);
                    }
                    if let Some(error) = &d.error {
                        pages::callout(ui, RED(), error);
                    }
                }
                pages::Part::Footer if s.applying => {
                    cancel = pages::running_footer(ui, widgets, &running, &["dialog-sign-stop"]);
                }
                pages::Part::Footer => {
                    if target.is_none() {
                        let r = pages::primary(
                            ui,
                            !busy && writable && s.settings.is_some(),
                            if s.reviewed {
                                "Save settings"
                            } else {
                                "Review settings"
                            },
                        );
                        control(widgets, "dialog-submit", &r, ui.clip_rect());
                        if r.clicked() {
                            if let Err(e) = s.options.validate() {
                                d.error = Some(user_error(&e));
                            } else if s.reviewed {
                                save = true;
                            } else {
                                s.reviewed = true;
                                d.error = None;
                            }
                        }
                        if s.reviewed {
                            let r = ui.add_enabled(!busy, Button::new("Edit"));
                            control(widgets, "dialog-sign-edit", &r, ui.clip_rect());
                            if r.clicked() {
                                s.reviewed = false;
                            }
                        }
                    }
                    let r = ui.add_enabled(
                        !busy,
                        Button::new(if target.is_some() {
                            "Verify again"
                        } else {
                            "Reload"
                        }),
                    );
                    control(widgets, "dialog-sign-reload", &r, ui.clip_rect());
                    refresh = r.clicked();
                    if busy {
                        let r = ui.button("Stop");
                        control(widgets, "dialog-sign-stop", &r, ui.clip_rect());
                        cancel = r.clicked();
                    }
                }
            },
        );
        close |= response.should_close() && !s.applying;
        if cancel {
            if s.applying {
                if let Some(write) = &mut self.write {
                    write.task.cancel();
                    write.cancelling = true;
                }
            } else {
                s.settings_task = None;
                s.verify_task = None;
                s.notice = Some("Cancelled. No new verification result or settings write.".into());
            }
        }
        let action = if save {
            s.applying = true;
            Some(Action::SetSigning {
                expected: std::sync::Arc::new(s.settings.clone().unwrap()),
                options: s.options.clone(),
            })
        } else {
            None
        };
        let owner = d.owner;
        if !close || s.applying {
            self.dialog = Some(d);
        }
        if refresh {
            self.start_signing_read();
        }
        if let (Some(action), Some(owner)) = (action, owner) {
            self.dispatch(Intent::Execute(owner, action));
            if self.write.is_none()
                && let Some(d) = &mut self.dialog
                && let Some(s) = &mut d.signing
            {
                s.applying = false;
                d.error = Some(
                    "Could not start. Wait for repository tasks to finish, then review again."
                        .into(),
                );
            }
        }
    }
}
