//! Update presentation. Network, verification and restart work stay in the controller.
use super::{Operation, State};
use crate::{app::Rocket, pages, style::*};
use eframe::egui::*;
use std::{collections::HashMap, sync::atomic::Ordering};

type Widgets = HashMap<String, crate::app::UiControl>;

#[derive(Default)]
struct Actions {
    close: bool,
    check: bool,
    download: bool,
    install: bool,
    stop: bool,
    browse: bool,
    folder: bool,
}

impl Rocket {
    pub(crate) fn update_status_ui(&mut self, ui: &mut Ui) {
        if self.updates.open || self.updates.release.is_none() {
            return;
        }
        let label = if self.updates.ready() {
            "Update ready"
        } else {
            "Update available"
        };
        let r = pages::tool(
            ui,
            Icon::Pull,
            label,
            !self.blocking_dialog() && !self.quitting,
        )
        .on_hover_text("Review the update without checking again");
        pages::control(&mut self.frame_widgets, "update-status", &r, ui.clip_rect());
        if r.clicked() {
            self.updates.reveal();
        }
    }

    fn update_notice_ui(&mut self, ctx: &Context) {
        if !self.updates.notice
            || self.blocking_dialog()
            || self.quitting
            || self.guide.is_some()
            || self.palette.open
            || Popup::is_any_open(ctx)
        {
            return;
        }
        let Some(release) = &self.updates.release else {
            return;
        };
        let mut review = false;
        let mut dismiss = false;
        Area::new(Id::new("rocket-update-notice"))
            .order(Order::Foreground)
            .anchor(Align2::RIGHT_TOP, vec2(-18.0, 122.0))
            .show(ctx, |ui| {
                Frame::new()
                    .fill(PANEL())
                    .stroke(Stroke::new(1.0, BORDER()))
                    .corner_radius(10)
                    .inner_margin(16)
                    .show(ui, |ui| {
                        ui.set_width((ctx.content_rect().width() - 84.0).clamp(240.0, 338.0));
                        ui.spacing_mut().item_spacing.y = 10.0;
                        ui.horizontal_top(|ui| {
                            let (tile, _) =
                                ui.allocate_exact_size(vec2(34.0, 34.0), Sense::hover());
                            ui.painter().rect_filled(tile, 9.0, ACCENT_SOFT());
                            icon(ui.painter(), tile.shrink(8.0), Icon::Pull, BLUE());
                            let width = (ui.available_width() - 36.0).max(100.0);
                            ui.vertical(|ui| {
                                ui.set_width(width);
                                ui.add(
                                    Label::new(
                                        RichText::new(format!(
                                            "Rocket {} is available",
                                            release.version.text()
                                        ))
                                        .font(semibold(14.0)),
                                    )
                                    .wrap(),
                                );
                                pages::note(ui, "A new version is ready to download.");
                            });
                            let r = crate::chrome::icon_button(ui, Icon::Close, true, false, 24.0)
                                .on_hover_text("Dismiss update notice");
                            pages::control(
                                &mut self.frame_widgets,
                                "update-notice-close",
                                &r,
                                ui.clip_rect(),
                            );
                            dismiss |= r.clicked();
                        });
                        ui.horizontal(|ui| {
                            ui.spacing_mut().interact_size.y = 30.0;
                            review = button(
                                ui,
                                &mut self.frame_widgets,
                                "update-notice-open",
                                "View update",
                                true,
                                true,
                            );
                            dismiss |= button(
                                ui,
                                &mut self.frame_widgets,
                                "update-notice-later",
                                "Not now",
                                true,
                                false,
                            );
                        });
                    });
            });
        if dismiss {
            self.updates.notice = false;
        }
        if review {
            self.updates.reveal();
        }
    }

    pub(crate) fn app_updates_ui(&mut self, ctx: &Context) {
        if !self.updates.open {
            self.update_notice_ui(ctx);
            return;
        }
        let can_install = self.write.is_none()
            && !self.external.running()
            && !self.forge.running()
            && !self.ssh.running()
            && !self.integrations.running();
        let mut action = dialog(ctx, &mut self.updates, &mut self.frame_widgets, can_install);
        if action.browse
            && let Some(release) = &self.updates.release
        {
            // The backend validates this exact GitHub release URL; notes are inert text.
            if let Some(urls) = &mut self.opened_urls {
                urls.push(release.page.clone());
            } else {
                ctx.open_url(OpenUrl::new_tab(release.page.clone()));
            }
        }
        if action.folder
            && let Some(prepared) = &self.updates.prepared
        {
            let folder = prepared.job.join("bundle");
            self.show_folder(&folder);
        }
        if action.close {
            if self.updates.running() {
                action.stop = true;
            }
            self.updates.open = false;
        }
        if action.stop {
            self.updates.stop();
            ctx.request_repaint();
        } else if !action.close {
            if action.check {
                self.start_update_check(ctx);
            }
            if action.download {
                self.download_update(ctx);
            }
            if action.install {
                self.install_update(ctx);
            }
        }
    }
}

fn dialog(ctx: &Context, s: &mut State, widgets: &mut Widgets, can_install: bool) -> Actions {
    let mut a = Actions::default();
    let width = (ctx.content_rect().width() - 48.0).clamp(300.0, 574.0);
    let expanded_heading = s.error.is_none() && matches!(s.status, "available" | "ready");
    let footer_id = Id::new((
        "update-footer-height",
        s.status,
        s.installable,
        width.to_bits(),
    ));
    let reserve: f32 = ctx.data(|d| d.get_temp(footer_id)).unwrap_or(128.0);
    let modal = Modal::new(Id::new("rocket-app-update"))
        .backdrop_color(Color32::from_black_alpha(125))
        .frame(
            Frame::new()
                .fill(PANEL())
                .stroke(Stroke::new(1.0, BORDER()))
                .corner_radius(12),
        )
        .show(ctx, |ui| {
            ui.set_width(width);
            ui.spacing_mut().item_spacing.y = 0.0;
            let header = Frame::new()
                .inner_margin(Margin {
                    left: 24,
                    right: 24,
                    top: 20,
                    bottom: 16,
                })
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.spacing_mut().item_spacing.y = 10.0;
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new("ROCKET UPDATES")
                                .size(11.0)
                                .color(HINT())
                                .strong(),
                        );
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let r = crate::chrome::icon_button(ui, Icon::Close, true, false, 26.0)
                                .on_hover_text(if s.running() {
                                    "Cancel and close (Esc)"
                                } else {
                                    "Close (Esc)"
                                });
                            pages::control(widgets, "update-close", &r, ui.clip_rect());
                            a.close = r.clicked();
                        });
                    });
                    if expanded_heading {
                        ui.add(Label::new(RichText::new(title(s)).font(semibold(24.0))).wrap());
                        pages::note(
                            ui,
                            if s.ready() {
                                "Your update is verified. Choose when to restart Rocket."
                            } else {
                                "Get the latest improvements, on your schedule."
                            },
                        );
                    }
                })
                .response
                .rect;
            // Reserve the measured header/footer before allocating the scroll area.
            // The conservative first-frame reserve also fits a wrapped footer hint.
            let max_body =
                (ctx.content_rect().height() - header.height() - reserve - 48.0).clamp(60.0, 620.0);
            let body = ScrollArea::vertical()
                .id_salt(("update-body", s.status))
                .max_height(max_body)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    Frame::new()
                        .inner_margin(Margin {
                            left: 24,
                            right: 24,
                            top: 4,
                            bottom: 24,
                        })
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.spacing_mut().item_spacing.y = 12.0;
                            body_ui(ui, s, widgets, &mut a, expanded_heading);
                        });
                });
            let body_rect = body
                .inner_rect
                .intersect(Rect::from_min_max(
                    body.inner_rect.min,
                    pos2(body.inner_rect.right(), ui.cursor().top()),
                ))
                .intersect(ui.clip_rect());
            pages::region(widgets, "update-scroll", body_rect, true);
            let footer = Frame::new()
                .fill(RAISED())
                .corner_radius(CornerRadius {
                    nw: 0,
                    ne: 0,
                    sw: 12,
                    se: 12,
                })
                .inner_margin(Margin {
                    left: 24,
                    right: 24,
                    top: 14,
                    bottom: 18,
                })
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.spacing_mut().item_spacing.y = 10.0;
                    footer_ui(ui, s, widgets, can_install, &mut a);
                })
                .response
                .rect;
            ui.painter()
                .hline(footer.x_range(), footer.top(), Stroke::new(1.0, HAIRLINE()));
            if (reserve - footer.height()).abs() > 0.5 {
                ctx.data_mut(|d| d.insert_temp(footer_id, footer.height()));
                ctx.request_discard("Update footer measurement");
            }
        });
    pages::region(widgets, "update-dialog", modal.response.rect, true);
    a.close |= modal.should_close();
    a
}

fn body_ui(
    ui: &mut Ui,
    s: &mut State,
    widgets: &mut Widgets,
    a: &mut Actions,
    expanded_heading: bool,
) {
    if !expanded_heading {
        ui.vertical_centered(|ui| {
            ui.add_space(12.0);
            if s.running() {
                ui.add(Spinner::new().size(44.0).color(BLUE()));
            } else {
                let (image, tint) = if s.error.is_some() {
                    (Icon::Warning, AMBER())
                } else if s.status == "current" {
                    (Icon::Check, GREEN())
                } else if s.status == "cancelled" {
                    (Icon::Stop, MUTED())
                } else {
                    (Icon::Info, BLUE())
                };
                let (rect, _) = ui.allocate_exact_size(vec2(54.0, 54.0), Sense::hover());
                ui.painter()
                    .circle_filled(rect.center(), 27.0, tint.gamma_multiply(0.12));
                icon(ui.painter(), rect.shrink(14.0), image, tint);
            }
            ui.add_space(8.0);
            ui.add(Label::new(RichText::new(title(s)).font(semibold(24.0))).wrap());
            ui.add(Label::new(RichText::new(description(s)).size(13.0).color(MUTED())).wrap());
            if s.release.is_none() {
                ui.add_space(4.0);
                ui.label(
                    RichText::new(format!(
                        "Installed {}   ·   Stable",
                        env!("CARGO_PKG_VERSION")
                    ))
                    .size(12.0)
                    .color(HINT()),
                );
            }
            ui.add_space(8.0);
        });
    }
    if s.error.is_some() {
        let r = pages::tool(
            ui,
            if s.details_expanded {
                Icon::Chevron
            } else {
                Icon::ChevronRight
            },
            "Error details",
            true,
        );
        pages::control(widgets, "update-error-details", &r, ui.clip_rect());
        if r.clicked() {
            s.details_expanded = !s.details_expanded;
        }
        if s.details_expanded {
            pages::well(ui, |ui| {
                ui.add(
                    Label::new(RichText::new(s.error.as_deref().unwrap_or_default()).size(12.0))
                        .wrap(),
                );
            });
        }
    } else if matches!(s.status, "available" | "ready") {
        versions(ui, s);
        if s.ready() {
            pages::well(ui, |ui| {
                ui.horizontal(|ui| {
                    let (rect, _) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
                    icon(ui.painter(), rect, Icon::Shield, GREEN());
                    ui.label(RichText::new("Download verified").font(semibold(13.0)));
                });
                pages::note(ui, "The previous installation is kept for recovery.");
                if !s.installable() {
                    pages::note(
                        ui,
                        "This development build cannot be replaced automatically. Open the download folder to use the packaged build.",
                    );
                }
            });
            a.folder = button(
                ui,
                widgets,
                "update-folder",
                "Open download folder",
                true,
                false,
            );
        } else if let Some(release) = &s.release {
            ui.label(
                RichText::new(format!(
                    "Windows x64   ·   {:.1} MB download",
                    release.size as f64 / 1_000_000.0
                ))
                .size(12.0)
                .color(HINT()),
            );
            ui.separator();
            ui.horizontal(|ui| {
                ui.label(RichText::new("What’s new").font(semibold(15.0)));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    a.browse = button(
                        ui,
                        widgets,
                        "update-release",
                        "View on GitHub ↗",
                        true,
                        false,
                    );
                });
            });
            if release.notes.trim().is_empty() {
                pages::note(ui, "No release notes were provided for this version.");
            } else {
                let r = pages::tool(
                    ui,
                    if s.notes_expanded {
                        Icon::Chevron
                    } else {
                        Icon::ChevronRight
                    },
                    if s.notes_expanded {
                        "Hide full release notes"
                    } else {
                        "Full release notes"
                    },
                    true,
                );
                pages::control(widgets, "update-notes", &r, ui.clip_rect());
                if r.clicked() {
                    s.notes_expanded = !s.notes_expanded;
                }
                if s.notes_expanded {
                    // Retain every character in full notes. URLs/markup never run code.
                    ui.add(
                        Label::new(RichText::new(&release.notes).size(13.0).color(TEXT_ITEM()))
                            .wrap(),
                    );
                } else {
                    for line in note_preview(&release.notes) {
                        ui.horizontal_top(|ui| {
                            ui.label(RichText::new("•").color(BLUE()));
                            ui.add(
                                Label::new(RichText::new(line).size(13.0).color(TEXT_ITEM()))
                                    .wrap(),
                            );
                        });
                    }
                }
            }
        }
    } else if s.running() && s.operation == Operation::Download {
        steps(ui, s);
    }
    if let Some(receipt) = &s.last_result {
        pages::well(ui, |ui| {
            pages::caption(ui, "Last update result");
            ui.add(Label::new(RichText::new(receipt).size(12.0)).wrap());
        });
    }
}

fn footer_ui(ui: &mut Ui, s: &State, widgets: &mut Widgets, can_install: bool, a: &mut Actions) {
    let running = s.running();
    let retry_download = s.release.is_some() && s.operation != Operation::Check;
    let hint = if running {
        "Your current installation stays in place."
    } else if s.ready() {
        "Save your work before restarting. Active terminals will close."
    } else if s.release.is_some() && s.error.is_none() && s.status != "cancelled" {
        "Download now. Choose when to restart."
    } else {
        "Checks run once at launch, or when you ask."
    };
    pages::note(ui, hint);
    ui.spacing_mut().interact_size.y = 34.0;
    ui.allocate_ui_with_layout(
        vec2(ui.available_width(), 34.0),
        Layout::right_to_left(Align::Center),
        |ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            if running {
                let label = if s.status == "stopping" {
                    "Stopping…"
                } else {
                    match s.operation {
                        Operation::Check => "Cancel check",
                        Operation::Download => "Cancel download",
                        Operation::Install => "Cancel restart",
                    }
                };
                a.stop = button(
                    ui,
                    widgets,
                    "update-stop",
                    label,
                    s.status != "stopping",
                    false,
                );
            } else {
                if s.error.is_some() || s.status == "cancelled" {
                    let retry = button(
                        ui,
                        widgets,
                        if retry_download {
                            "update-download"
                        } else {
                            "update-check"
                        },
                        "Try again",
                        s.endpoint.is_some(),
                        true,
                    );
                    if retry_download {
                        a.download = retry;
                    } else {
                        a.check = retry;
                    }
                } else if s.ready() && s.installable() {
                    a.install = button(
                        ui,
                        widgets,
                        "update-install",
                        "Install and restart",
                        can_install,
                        true,
                    );
                } else if s.status == "available" && s.release.is_some() {
                    a.download = button(
                        ui,
                        widgets,
                        "update-download",
                        "Download update",
                        true,
                        true,
                    );
                } else if !s.ready() {
                    a.check = button(
                        ui,
                        widgets,
                        "update-check",
                        "Check again",
                        s.endpoint.is_some(),
                        true,
                    );
                }
                a.close |= button(
                    ui,
                    widgets,
                    "update-later",
                    if matches!(s.status, "available" | "ready") {
                        "Later"
                    } else if s.status == "current" {
                        "Done"
                    } else {
                        "Close"
                    },
                    true,
                    false,
                );
            }
        },
    );
    if s.ready() && s.installable() && !can_install {
        pages::note(ui, "Finish the running operation before installing.");
    }
}

fn title(s: &State) -> String {
    if s.error.is_some() {
        return match s.operation {
            Operation::Check => "Couldn’t check for updates",
            Operation::Download => "Couldn’t prepare the update",
            Operation::Install => "Couldn’t prepare the restart",
        }
        .into();
    }
    match s.phase() {
        "available" => {
            return format!(
                "Rocket {} is available",
                s.release
                    .as_ref()
                    .map(|r| r.version.text())
                    .unwrap_or_default()
            );
        }
        "ready" => "Ready when you are",
        "current" => "You’re up to date",
        "unavailable" => "No public release yet",
        "disabled" => "Updates are unavailable",
        "downloading" => "Downloading your update",
        "verifying_archive" | "verifying_files" => "Verifying your update",
        "preparing_restart" => "Preparing to restart",
        "stopping" => "Stopping…",
        "cancelled" => {
            if s.operation == Operation::Install {
                "Restart cancelled"
            } else {
                "Update cancelled"
            }
        }
        _ => "Checking for updates",
    }
    .into()
}

fn description(s: &State) -> &str {
    if s.error.is_some() {
        // Preserve the real error in a short preview and in full under Error details.
        let error = s
            .error
            .as_deref()
            .unwrap_or_default()
            .lines()
            .next()
            .unwrap_or("The operation failed. See Error details.");
        return error.strip_prefix("invalid input: ").unwrap_or(error);
    }
    match s.phase() {
        "checking" | "idle" => "Looking for the latest Rocket release on GitHub.",
        "downloading" => "Downloading the release package. You can keep your current version.",
        "verifying_archive" => "Checking the download hash and unpacking the release.",
        "verifying_files" => "Checking the files in the new installation.",
        "preparing_restart" => {
            "Preparing the verified package. Rocket will check for unsaved work before closing."
        }
        "stopping" => "Waiting for the update worker to stop and release its files.",
        "current" => "You have the latest available stable version of Rocket.",
        _ => s
            .message
            .as_deref()
            .unwrap_or("Review the update before continuing."),
    }
}

fn versions(ui: &mut Ui, s: &State) {
    ui.horizontal(|ui| {
        let width = ((ui.available_width() - 40.0) / 2.0).max(90.0);
        version_card(ui, width, "Installed", env!("CARGO_PKG_VERSION"), false);
        let (rect, _) = ui.allocate_exact_size(vec2(20.0, 20.0), Sense::hover());
        icon(ui.painter(), rect.shrink(3.0), Icon::ChevronRight, HINT());
        let version = s
            .release
            .as_ref()
            .map(|r| r.version.text())
            .unwrap_or_default();
        version_card(
            ui,
            width,
            if s.ready() {
                "Ready to install"
            } else {
                "Available"
            },
            &version,
            true,
        );
    });
}

fn version_card(ui: &mut Ui, width: f32, label: &str, version: &str, available: bool) {
    Frame::new()
        .fill(if available { ACCENT_SOFT() } else { BG() })
        .stroke(Stroke::new(
            1.0,
            if available {
                BLUE().gamma_multiply(0.35)
            } else {
                BORDER()
            },
        ))
        .corner_radius(8)
        .inner_margin(14)
        .show(ui, |ui| {
            ui.vertical(|ui| {
                ui.set_width((width - 30.0).max(60.0));
                ui.spacing_mut().item_spacing.y = 6.0;
                ui.label(RichText::new(label).size(12.0).color(MUTED()));
                ui.add(
                    Label::new(
                        RichText::new(version)
                            .font(semibold(21.0))
                            .color(if available { BLUE() } else { TEXT() }),
                    )
                    .wrap(),
                );
                ui.label(RichText::new("Stable").size(11.0).color(HINT()));
            });
        });
}

fn steps(ui: &mut Ui, s: &State) {
    let active = if s.progress.load(Ordering::Acquire) < 2 {
        0
    } else {
        1
    };
    ui.columns(3, |cols| {
        for (i, (col, label)) in cols
            .iter_mut()
            .zip(["Download", "Verify", "Ready"])
            .enumerate()
        {
            col.vertical_centered(|ui| {
                let tint = if i <= active { BLUE() } else { FAINT() };
                let (rect, _) =
                    ui.allocate_exact_size(vec2(ui.available_width(), 3.0), Sense::hover());
                ui.painter().rect_filled(rect, 2.0, tint);
                ui.label(RichText::new(label).size(12.0).color(tint));
            });
        }
    });
}

fn button(
    ui: &mut Ui,
    widgets: &mut Widgets,
    id: &str,
    text: &str,
    enabled: bool,
    primary: bool,
) -> bool {
    let r = ui
        .push_id(id, |ui| {
            if primary {
                pages::primary(ui, enabled, text)
            } else {
                ui.add_enabled(enabled, Button::new(text))
            }
        })
        .inner;
    pages::control(widgets, id, &r, ui.clip_rect());
    r.clicked()
}

/// Show excerpts from the release itself, never synthetic "highlights". The
/// disclosure above these excerpts always retains the complete original notes.
fn note_preview(notes: &str) -> Vec<String> {
    let lines: Vec<_> = notes
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .take(3)
        .collect();
    let lines = if lines.is_empty() {
        vec![notes.trim()]
    } else {
        lines
    };
    lines
        .into_iter()
        .map(|line| {
            let line = line
                .strip_prefix("- ")
                .or_else(|| line.strip_prefix("* "))
                .unwrap_or(line);
            let mut chars = line.chars();
            let mut excerpt: String = chars.by_ref().take(220).collect();
            if chars.next().is_some() {
                excerpt.push('…');
            }
            excerpt
        })
        .collect()
}
