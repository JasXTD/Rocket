//! The Security overview and its local navigation. Scan and write policy stay in their controllers.
use super::{Rocket, scan};
use crate::{pages, preferences, style::*};
use eframe::egui::*;

impl Rocket {
    pub(crate) fn security_page(&mut self, ui: &mut Ui) {
        let owner = self.tab().map(|t| t.id);
        if self.dependencies.setup_page {
            let back = ui.add(
                Button::new(RichText::new("‹ Back to protections").color(BLUE())).frame(false),
            );
            pages::control(
                &mut self.frame_widgets,
                "security-overview",
                &back,
                ui.clip_rect(),
            );
            if back.clicked() {
                self.dependencies.setup_page = false;
            }
            ui.add_space(8.0);
            let (scan, update) = self.dependency_setup_page(ui, owner);
            self.dependency_notices(ui, owner);
            if scan {
                self.start_dependency_scan(true, false);
            }
            if update {
                self.start_dependency_scan(true, true);
            }
            return;
        }

        let protection_editable = self.secret_protection_editable && !self.busy();
        preferences::card_frame(ui, |ui| {
            let changed = protection_header(ui, "Secret protection", |ui| {
                let response = ui
                    .add_enabled_ui(protection_editable, |ui| {
                        switch(ui, &mut self.secret_protection, "Enable secret protection")
                    })
                    .inner;
                pages::control(
                    &mut self.frame_widgets,
                    "pref-secret-protection",
                    &response,
                    ui.clip_rect(),
                );
                response.changed()
            });
            if changed && let Some(backend) = &self.backend {
                backend.set_secret_protection(self.secret_protection);
            }
            preferences::card_note(ui, |ui| {
                status(
                    ui,
                    if self.secret_protection {
                        "Blocks commits & pushes"
                    } else {
                        "Protection is off"
                    },
                    if self.secret_protection {
                        GREEN()
                    } else {
                        AMBER()
                    },
                );
                help(
                    ui,
                    "Checks credentials in messages, staged changes and outgoing history.",
                );
                if !self.secret_protection {
                    help(
                        ui,
                        "General secret checks are off. The AI API-key guard has its own switch below.",
                    );
                }
                if !self.secret_protection_editable {
                    help(
                        ui,
                        "Only the user can change this in the normal Rocket application.",
                    );
                }
            });
            preferences::divider(ui);
            preferences::card_note(ui, |ui| {
                let response = ui.collapsing("What gets checked?", |ui| {
                    help(ui, "Passwords, private keys, supported tokens and sensitive .env files. Valid environment placeholders remain allowed.");
                });
                pages::control(
                    &mut self.frame_widgets,
                    "security-secret-coverage",
                    &response.header_response,
                    ui.clip_rect(),
                );
            });
        });
        preferences::card_frame(ui, |ui| {
            let changed = protection_header(ui, "Guard AI API keys", |ui| {
                let response = ui
                    .add_enabled_ui(protection_editable, |ui| {
                        switch(ui, &mut self.guard_ai_api_keys, "Enable AI API-key guard")
                    })
                    .inner;
                pages::control(
                    &mut self.frame_widgets,
                    "pref-ai-key-guard",
                    &response,
                    ui.clip_rect(),
                );
                response.changed()
            });
            if changed && let Some(backend) = &self.backend {
                backend.set_guard_ai_api_keys(self.guard_ai_api_keys);
            }
            preferences::card_note(ui, |ui| {
                status(
                    ui,
                    if self.guard_ai_api_keys {
                        "Blocks exposed AI API keys"
                    } else {
                        "AI key guard is off"
                    },
                    if self.guard_ai_api_keys {
                        GREEN()
                    } else {
                        AMBER()
                    },
                );
                help(
                    ui,
                    "Checks known key formats in messages, staged files and outgoing history. Runs locally, without contacting providers.",
                );
                if !self.guard_ai_api_keys {
                    help(
                        ui,
                        "Provider-format checks are off. Secret protection may still block credential assignments.",
                    );
                }
                if !self.secret_protection_editable {
                    help(
                        ui,
                        "Only the user can change this in the normal Rocket application.",
                    );
                }
            });
            preferences::divider(ui);
            preferences::card_note(ui, |ui| {
                let response = ui.collapsing("Supported key formats", |ui| {
                    help(ui, "OpenAI, Anthropic, Groq, xAI, OpenRouter, Hugging Face and standard Google API keys (also used by Gemini).");
                    help(ui, "DeepSeek and GLM / Z.ai keys are checked in credential assignments such as DEEPSEEK_API_KEY, ZAI_API_KEY or api_key, avoiding ordinary hashes and dotted IDs.");
                    help(ui, "Matches known formats, not every provider or future key version. Placeholders remain allowed; realistic synthetic keys are still blocked.");
                });
                pages::control(
                    &mut self.frame_widgets,
                    "security-ai-key-coverage",
                    &response.header_response,
                    ui.clip_rect(),
                );
            });
        });
        self.scan_budget_card(ui);
        self.dependency_page(ui);
        self.sync_dependency_push_check();
        preferences::status_line(
            ui,
            MUTED(),
            &format!(
                "When you push: {} {} {}",
                if self.secret_protection {
                    "Secrets are checked first."
                } else {
                    "Secret protection is off."
                },
                if self.guard_ai_api_keys {
                    "AI key formats are checked."
                } else {
                    "AI key guard is off."
                },
                if self.dependencies.before_push {
                    "OSV scans the pushed tips; warnings pause the push for review."
                } else {
                    "Before-push OSV scanning is off."
                }
            ),
        );
    }

    fn dependency_notices(&self, ui: &mut Ui, owner: Option<u64>) {
        if let Some(notice) = &self.dependencies.update_notice {
            pages::callout(ui, BLUE(), notice);
        }
        if let Some(error) = owner
            .and_then(|id| self.dependencies.repositories.get(&id))
            .and_then(|s| s.error.as_ref())
        {
            pages::callout(ui, AMBER(), error);
        }
    }

    fn dependency_page(&mut self, ui: &mut Ui) {
        let owner = self.tab().map(|t| t.id);
        let enabled_before = self.dependencies.enabled;
        let editable = self.dependencies.editable;
        let push_editable = editable && !self.working();
        let mut scan_now = false;
        preferences::card_frame(ui, |ui| {
            protection_header(ui, "Dependency checks", |ui| {
                let response = ui
                    .add_enabled_ui(editable, |ui| {
                        switch(
                            ui,
                            &mut self.dependencies.enabled,
                            "Enable background dependency checks",
                        )
                    })
                    .inner;
                pages::control(
                    &mut self.frame_widgets,
                    "pref-dependency-warnings",
                    &response,
                    ui.clip_rect(),
                );
            });
            // Keep the existing disable/re-enable lifecycle: cancelled results must not
            // repopulate the cache after the user turns background checks off.
            if enabled_before != self.dependencies.enabled {
                if enabled_before && let Some(job) = &mut self.dependencies.job {
                    job.discard = true;
                    job.task.cancel();
                }
                self.dependencies.repositories.clear();
            }
            preferences::card_note(ui, |ui| {
                status(ui, "Background scans · warnings only", MUTED());
                help(
                    ui,
                    "Checks dependency versions against local OSV advisory data.",
                );
                if !editable {
                    help(
                        ui,
                        "Only the user can change this preference or download advisory data in the normal Rocket application.",
                    );
                }
            });
            preferences::divider(ui);
            ui.add_enabled_ui(push_editable, |ui| {
                preferences::bar_switch_row(
                    ui,
                    &mut self.frame_widgets,
                    "pref-dependency-push",
                    &mut self.dependencies.before_push,
                    "Scan dependencies before pushing",
                    "Review warnings before Git publishes the committed versions.",
                    "",
                );
            });
            preferences::divider(ui);
            preferences::card_note(ui, |ui| {
                let repository = self.tab().map(|t| {
                    t.path
                        .file_name()
                        .unwrap_or(t.path.as_os_str())
                        .to_string_lossy()
                        .into_owned()
                });
                ui.label(
                    RichText::new(repository.unwrap_or_else(|| "No repository selected".into()))
                        .font(semibold(12.0)),
                );
                ui.add_space(5.0);
                let state = owner.and_then(|id| self.dependencies.repositories.get(&id));
                let report = state
                    .and_then(|s| s.cached.as_ref())
                    .map(|c| c.report.clone());
                let error = state.is_some_and(|s| s.error.is_some());
                let running = self.dependencies.running();
                let ready = self.dependencies.setup.ready;
                let (title, tint) = if running {
                    let title = if self.dependencies.setup.running() {
                        "Checking the scanner…"
                    } else if self
                        .dependencies
                        .job
                        .as_ref()
                        .is_some_and(|job| Some(job.owner) != owner)
                    {
                        "Checking another repository…"
                    } else if self.dependencies.job.as_ref().is_some_and(|job| job.update) {
                        "Downloading advisory data…"
                    } else {
                        "Scanning dependencies…"
                    };
                    (title.into(), BLUE())
                } else if !ready {
                    (
                        if self.dependencies.setup.error.is_some() {
                            "Scanner needs attention"
                        } else {
                            "Set up dependency checks"
                        }
                        .into(),
                        MUTED(),
                    )
                } else if owner.is_none() {
                    ("Open a repository to scan".into(), MUTED())
                } else if error {
                    ("Scan incomplete".into(), AMBER())
                } else if let Some(report) = &report {
                    report_heading(report)
                } else if !self.dependencies.enabled {
                    ("Background checks are off".into(), MUTED())
                } else {
                    ("Not scanned yet".into(), MUTED())
                };
                let description = if let Some(report) = &report {
                    format!(
                        "{} packages · {} files · checked {}",
                        report.packages,
                        report.files,
                        ago(
                            scan::now().saturating_sub(report.checked),
                            report.checked as i64
                        )
                    )
                } else {
                    if !ready {
                        "Choose the scanner, then download advisory data."
                    } else if owner.is_none() {
                        "Scanner setup is available for your next repository."
                    } else {
                        "Run an offline scan using the advisory data in this profile."
                    }
                    .into()
                };
                result_row(ui, &title, tint, &description, |ui| {
                    if running {
                        ui.spinner();
                        let response = ui.button("Cancel");
                        pages::control(
                            &mut self.frame_widgets,
                            "dependencies-cancel",
                            &response,
                            ui.clip_rect(),
                        );
                        if response.clicked() {
                            self.dependencies.cancel();
                        }
                    } else if !ready {
                        let response = pages::primary(
                            ui,
                            true,
                            if self.dependencies.setup.error.is_some() {
                                "Review setup"
                            } else {
                                "Set up scanner"
                            },
                        );
                        pages::control(
                            &mut self.frame_widgets,
                            "dependency-setup-start",
                            &response,
                            ui.clip_rect(),
                        );
                        if response.clicked() {
                            self.dependencies.setup_page = true;
                        }
                    } else {
                        let response = pages::primary(
                            ui,
                            owner.is_some(),
                            if error { "Retry scan" } else { "Scan now" },
                        );
                        pages::control(
                            &mut self.frame_widgets,
                            "dependencies-scan",
                            &response,
                            ui.clip_rect(),
                        );
                        scan_now = response.clicked();
                    }
                });
                if let Some(error) = self.dependencies.setup.error.as_ref() {
                    pages::callout(ui, AMBER(), error);
                }
                self.dependency_notices(ui, owner);
                if let Some(report) = &report {
                    ui.add_space(7.0);
                    let age = scan::now().saturating_sub(report.oldest_database) / 86_400;
                    if age > 7 {
                        preferences::status_line(
                            ui,
                            AMBER(),
                            &format!(
                                "Advisory data is {age} days old. Update it in Scanner & data to check newer advisories."
                            ),
                        );
                    }
                    if !report.notes.is_empty() {
                        let response = ui.collapsing(
                            format!("Coverage notes ({})", report.notes.len()),
                            |ui| {
                                for note in &report.notes {
                                    help(ui, note);
                                }
                            },
                        );
                        pages::control(
                            &mut self.frame_widgets,
                            "dependency-coverage",
                            &response.header_response,
                            ui.clip_rect(),
                        );
                    }
                    if report.findings.is_empty() && !running && !error {
                        help(
                            ui,
                            "No matches in the local database. Coverage does not guarantee safety.",
                        );
                    }
                    for (index, finding) in report.findings.iter().enumerate() {
                        preferences::divider(ui);
                        let response = CollapsingHeader::new(
                            RichText::new(format!(
                                "{} · {} {}",
                                finding.severity, finding.package, finding.version
                            ))
                            .font(semibold(12.0)),
                        )
                        .id_salt((
                            "security-finding",
                            owner,
                            &finding.ids,
                            &finding.package,
                            &finding.file,
                        ))
                        .show(ui, |ui| {
                            ui.add(Label::new(&finding.summary).wrap());
                            help(ui, &format!("{} · {}", finding.ecosystem, finding.file));
                            help(ui, "Application reachability has not been checked.");
                            ui.horizontal_wrapped(|ui| {
                                for (n, id) in finding.ids.iter().enumerate() {
                                    let response = ui.hyperlink_to(
                                        id,
                                        format!("https://osv.dev/vulnerability/{id}"),
                                    );
                                    pages::control(
                                        &mut self.frame_widgets,
                                        format!("dependency-advisory-{index}-{n}"),
                                        &response,
                                        ui.clip_rect(),
                                    );
                                }
                            });
                        });
                        pages::control(
                            &mut self.frame_widgets,
                            format!("dependency-finding-{index}"),
                            &response.header_response,
                            ui.clip_rect(),
                        );
                        help(
                            ui,
                            &format!(
                                "{} · {}",
                                if finding.fixed.is_empty() {
                                    "No fixed version published".into()
                                } else {
                                    format!("Reported fix: {}", finding.fixed.join(", "))
                                },
                                finding.file
                            ),
                        );
                        ui.add_space(5.0);
                    }
                }
            });
            preferences::divider(ui);
            preferences::card_note(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    help(
                        ui,
                        &if self.dependencies.setup.ready {
                            format!("OSV {} verified", rocket_git::OSV_VERSION)
                        } else {
                            "Scanner not configured".into()
                        },
                    );
                    let response = ui.add(
                        Button::new(RichText::new("Scanner & data").color(BLUE())).frame(false),
                    );
                    pages::control(
                        &mut self.frame_widgets,
                        "dependency-setup-open",
                        &response,
                        ui.clip_rect(),
                    );
                    if response.clicked() {
                        self.dependencies.setup_page = true;
                    }
                });
            });
        });
        if scan_now {
            self.start_dependency_scan(true, false);
        }
    }
}

/// Use natural-height text and the existing native switch rather than a fixed-height
/// label row: translated text, long names and the compact Preferences rail must wrap.
fn protection_header<R>(ui: &mut Ui, title: &str, toggle: impl FnOnce(&mut Ui) -> R) -> R {
    Frame::new()
        .inner_margin(Margin {
            left: 14,
            right: 14,
            top: 14,
            bottom: 0,
        })
        .show(ui, |ui| {
            let width = (ui.available_width() - 50.0).max(60.0);
            ui.horizontal_top(|ui| {
                ui.allocate_ui_with_layout(vec2(width, 20.0), Layout::top_down(Align::Min), |ui| {
                    ui.set_width(width);
                    ui.add(Label::new(RichText::new(title).font(semibold(13.0))).wrap());
                });
                ui.allocate_ui_with_layout(
                    vec2(42.0, 20.0),
                    Layout::right_to_left(Align::Center),
                    toggle,
                )
                .inner
            })
            .inner
        })
        .inner
}
fn help(ui: &mut Ui, text: &str) {
    ui.add(Label::new(RichText::new(text).size(12.0).color(MUTED())).wrap());
}
fn result_row(
    ui: &mut Ui,
    title: &str,
    tint: Color32,
    description: &str,
    action: impl FnOnce(&mut Ui),
) {
    let summary = |ui: &mut Ui| {
        ui.add(Label::new(RichText::new(title).font(semibold(15.0)).color(tint)).wrap());
        help(ui, description);
    };
    if ui.available_width() < 400.0 {
        summary(ui);
        ui.add_space(6.0);
        ui.horizontal_wrapped(action);
    } else {
        let width = ui.available_width() - 128.0;
        ui.horizontal_top(|ui| {
            ui.allocate_ui_with_layout(vec2(width, 0.0), Layout::top_down(Align::Min), |ui| {
                ui.set_width(width);
                summary(ui);
            });
            ui.allocate_ui_with_layout(
                vec2(120.0, 26.0),
                Layout::right_to_left(Align::Min),
                action,
            );
        });
    }
}
fn status(ui: &mut Ui, text: &str, color: Color32) {
    ui.label(RichText::new(text).size(12.0).color(color));
}
fn report_heading(report: &scan::Report) -> (String, Color32) {
    match report.findings.len() {
        0 if !report.notes.is_empty() => ("No advisory matches · coverage limited".into(), AMBER()),
        0 if report.packages == 0 => ("No dependencies identified".into(), MUTED()),
        0 => ("No known advisories found".into(), GREEN()),
        1 => ("1 advisory needs attention".into(), AMBER()),
        count => (format!("{count} advisories need attention"), AMBER()),
    }
}
