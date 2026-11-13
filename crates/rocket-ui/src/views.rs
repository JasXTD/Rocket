use crate::{
    app::*,
    backend::Action,
    i18n::{tr, trf, trn},
    pages::{self, Side},
    patch_view::patch_view,
    style::{self, *},
};
use eframe::egui::*;
use rocket_git::*;
impl Rocket {
    /// No repository open: Open / Clone / New cards, the drop hint, recent
    /// repositories and the Git discovery status.
    pub(crate) fn welcome(&mut self, ui: &mut Ui, intents: &mut Vec<Intent>) {
        let ready = self.backend.is_some();
        let dropping = ui.input(|i| !i.raw.hovered_files.is_empty());
        let top = (ui.available_height() * 0.12).clamp(24.0, 96.0);
        ScrollArea::vertical()
            .id_salt("welcome")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let width = (ui.available_width() - 48.0).clamp(300.0, 640.0);
                ui.add_space(top);
                ui.horizontal(|ui| {
                    ui.add_space(((ui.available_width() - width) / 2.0).max(24.0));
                    ui.vertical(|ui| {
                        ui.set_width(width);
                        ui.spacing_mut().item_spacing.y = 4.0;
                        ui.label(RichText::new(tr("Open a repository")).font(semibold(22.0)));
                        ui.label(
                            RichText::new(
                                tr("Each repository opens in its own tab. Nothing changes until you act."),
                            )
                            .color(MUTED()),
                        );
                        ui.add_space(18.0);
                        let guided = ui.button(tr("New to Rocket? Setup Guide"));
                        pages::control(&mut self.frame_widgets, "welcome-setup", &guided, ui.clip_rect());
                        if guided.clicked() { intents.push(Intent::GuidedSetup); }
                        ui.add_space(10.0);
                        let gap = 12.0;
                        let card = ((width - 2.0 * gap) / 3.0).floor();
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = gap;
                            for (image, title, detail, shortcut, key, enabled, kind) in [
                                (
                                    Icon::Folder,
                                    tr("Open folder"),
                                    tr("Existing repository"),
                                    "Ctrl+O",
                                    "open",
                                    ready,
                                    DialogKind::Open,
                                ),
                                (
                                    Icon::Fetch,
                                    tr("Clone"),
                                    tr("From a URL or path"),
                                    "Ctrl+Shift+O",
                                    "repository-clone-welcome",
                                    ready,
                                    DialogKind::Clone,
                                ),
                                (
                                    Icon::Plus,
                                    tr("New repository"),
                                    tr("Start tracking a project"),
                                    "Ctrl+N",
                                    "repository-new-welcome",
                                    ready && self.write.is_none(),
                                    DialogKind::NewRepository,
                                ),
                            ] {
                                let r = start_card(ui, card, image, title, detail, shortcut, enabled);
                                pages::control(&mut self.frame_widgets, key, &r, ui.clip_rect());
                                if r.clicked() {
                                    intents.push(Intent::Dialog(kind));
                                }
                            }
                        });
                        ui.add_space(16.0);
                        pages::caption(ui, tr("Explore"));
                        ui.add_space(4.0);
                        // Enter opens Explore with this search or address.
                        let explore = ui.add_enabled(
                            ready,
                            TextEdit::singleline(&mut self.explore.query)
                                .hint_text(tr("Search GitHub or paste a repository address"))
                                .desired_width(width)
                                .margin(Margin::symmetric(10, 7)),
                        );
                        pages::control(&mut self.frame_widgets, "welcome-explore", &explore, ui.clip_rect());
                        if explore.lost_focus()
                            && ui.input(|i| i.key_pressed(Key::Enter))
                            && !self.explore.query.trim().is_empty()
                        {
                            intents.push(Intent::Explore(Some(self.explore.query.clone())));
                        }
                        ui.add_space(12.0);
                        // Dropping works anywhere in the window; the hint lights up while
                        // a folder is dragged over it.
                        let (zone, _) =
                            ui.allocate_exact_size(vec2(width, 40.0), Sense::hover());
                        if dropping {
                            ui.painter().rect_filled(zone, 8.0, ACCENT_SOFT());
                        }
                        ui.painter().rect_stroke(
                            zone,
                            8.0,
                            Stroke::new(1.0, if dropping { BLUE() } else { BORDER() }),
                            StrokeKind::Inside,
                        );
                        ui.painter().text(
                            zone.center(),
                            Align2::CENTER_CENTER,
                            if dropping {
                                tr("Release to open this folder")
                            } else {
                                tr("or drop a repository folder anywhere in this window")
                            },
                            FontId::proportional(12.5),
                            if dropping { BLUE() } else { FAINT() },
                        );
                        if !self.recent.is_empty() {
                            ui.add_space(22.0);
                            ui.horizontal(|ui| {
                                pages::caption(ui, tr("Recent"));
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    ui.label(
                                        RichText::new(tr("Click to open")).size(11.5).color(FAINT()),
                                    );
                                });
                            });
                            ui.spacing_mut().item_spacing.y = 0.0;
                            for (i, path) in self.recent.iter().enumerate() {
                                let r = recent_row(ui, path, self.recent_heads.get(path));
                                pages::control(
                                    &mut self.frame_widgets,
                                    format!("welcome-recent-{i}"),
                                    &r,
                                    ui.clip_rect(),
                                );
                                if r.clicked() {
                                    intents.push(Intent::Open(path.clone()));
                                }
                            }
                            ui.spacing_mut().item_spacing.y = 4.0;
                        }
                        ui.add_space(20.0);
                        if let Some(error) = &self.startup_error {
                            pages::callout(ui, RED(), error);
                            ui.add_space(4.0);
                            let retry = ui.button(tr("Retry Git discovery"));
                            pages::control(
                                &mut self.frame_widgets,
                                "welcome-git-retry",
                                &retry,
                                ui.clip_rect(),
                            );
                            if retry.clicked() {
                                self.start_backend();
                            }
                        } else if let Some(backend) = &self.backend {
                            ui.horizontal(|ui| {
                                let (mark, _) =
                                    ui.allocate_exact_size(vec2(12.0, 12.0), Sense::hover());
                                icon(ui.painter(), mark, Icon::Check, GREEN());
                                let version = backend
                                    .version
                                    .strip_prefix("git version ")
                                    .unwrap_or(&backend.version);
                                ui.label(
                                    RichText::new(trf!("Git {version} found", version))
                                        .size(12.0)
                                        .color(FAINT()),
                                )
                                .on_hover_text(backend.executable.display().to_string());
                            });
                        } else {
                            ui.horizontal(|ui| {
                                ui.spinner();
                                ui.label(RichText::new(tr("Finding Git…")).size(12.0).color(MUTED()));
                            });
                        }
                    });
                });
            });
    }
    pub(crate) fn activity(&mut self, ui: &mut Ui) {
        self.drawer_header(ui, true);
        self.activity_rows(ui);
    }
    /// Activity's actions in the drawer header, added right to left.
    pub(crate) fn activity_actions(&mut self, ui: &mut Ui) {
        if self.write.is_none() {
            let clear = pages::tool(ui, Icon::Trash, tr("Clear"), !self.activities.is_empty())
                .on_hover_text(tr("Clear this list; diagnostics keep the records"));
            self.frame_widgets
                .insert("console-clear".into(), (&clear).into());
            if clear.clicked() {
                self.activities.clear();
            }
        }
        let history = pages::tool(ui, Icon::External, tr("All diagnostics"), true);
        self.frame_widgets
            .insert("activity-open".into(), (&history).into());
        if history.clicked() {
            self.activity_history.show();
            self.external.open = false;
        }
    }
    /// Drawer rows, newest first: when, status, operation, summary, duration and
    /// details on demand. A running operation stays on top with its progress.
    fn activity_rows(&mut self, ui: &mut Ui) {
        ui.spacing_mut().item_spacing.y = 0.0;
        // With its dialog open, the operation's spinner and Stop are in the dialog's
        // footer and this row only names it.
        let quiet = self.operation_in_dialog();
        if let Some(write) = &self.write {
            let mut cancel = None;
            Frame::new()
                .fill(BLUE().gamma_multiply(0.05))
                .inner_margin(Margin {
                    left: 16,
                    right: 12,
                    top: 2,
                    bottom: 8,
                })
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.set_height(34.0);
                        let (when, _) = ui.allocate_exact_size(vec2(68.0, 34.0), Sense::hover());
                        ui.painter().text(
                            when.left_center(),
                            Align2::LEFT_CENTER,
                            tr("now"),
                            FontId::proportional(11.5),
                            HINT(),
                        );
                        if quiet {
                            let (dot, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
                            ui.painter().circle_filled(dot.center(), 3.5, BLUE());
                        } else {
                            ui.add(Spinner::new().size(14.0));
                        }
                        ui.add_space(6.0);
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if !quiet {
                                cancel = Some(pages::danger_ghost(
                                    ui,
                                    write.can_cancel(),
                                    write.cancel_label(),
                                ));
                            }
                            let progress = match write.fraction {
                                Some(f) => {
                                    trf!("{progress} · current phase {percent}%", progress = write.progress, percent = format!("{:.0}", f * 100.0))
                                }
                                None => write.progress.clone(),
                            };
                            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                                let name = ui.label(RichText::new(&write.title).font(semibold(12.5)));
                                if quiet {
                                    name.on_hover_text(progress);
                                } else {
                                    ui.add(
                                        Label::new(RichText::new(progress).size(12.0).color(MUTED()))
                                            .truncate(),
                                    );
                                }
                            });
                        });
                    });
                    if let Some(fraction) = write.fraction.filter(|_| !quiet) {
                        ui.horizontal(|ui| {
                            ui.add_space(90.0);
                            ui.add(ProgressBar::new(fraction).desired_height(3.0).fill(BLUE()));
                        });
                    }
                });
            let line = ui.cursor().top();
            ui.painter()
                .hline(ui.max_rect().x_range(), line, Stroke::new(1.0, HAIRLINE()));
            if let Some(cancel) = cancel {
                self.frame_widgets
                    .insert("cancel-operation".into(), (&cancel).into());
                if cancel.clicked() {
                    self.cancel_write();
                }
            }
        }
        ScrollArea::vertical()
            .id_salt("activity-rows")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                if self.activities.is_empty() && self.write.is_none() {
                    ui.vertical_centered(|ui| {
                        ui.add_space(26.0);
                        let (rect, _) = ui.allocate_exact_size(vec2(20.0, 20.0), Sense::hover());
                        icon(ui.painter(), rect, Icon::Activity, HINT());
                        ui.add_space(6.0);
                        ui.label(RichText::new(tr("No operations yet")).font(semibold(13.0)));
                        ui.label(
                            RichText::new(tr("Git operations appear here as you run them. F5 refreshes."))
                                .size(12.0)
                                .color(HINT()),
                        );
                    });
                }
                let now = std::time::SystemTime::now();
                for (i, activity) in self.activities.iter().enumerate().rev() {
                    ui.push_id(i, |ui| {
                        activity_row(ui, i, activity, now, &mut self.frame_widgets)
                    });
                }
            });
    }
}

/// One Activity drawer row. Details stay folded until asked for; the fold state
/// follows the activity rather than its position, which shifts as old rows drop.
fn activity_row(
    ui: &mut Ui,
    index: usize,
    activity: &Activity,
    now: std::time::SystemTime,
    widgets: &mut std::collections::HashMap<String, UiControl>,
) {
    let outcome = &activity.outcome;
    let status = if outcome.paused {
        pages::Status::Review
    } else if !outcome.command_succeeded {
        pages::Status::Failed
    } else if outcome.recovery_pending {
        pages::Status::Review
    } else {
        pages::Status::Done
    };
    let has_details = !outcome.details.is_empty()
        || outcome.retained_workspace.is_some()
        || !outcome.raw_output.is_empty();
    let fold = Id::unique((
        "activity-details",
        &activity.title,
        &outcome.summary,
        activity.elapsed,
    ));
    let raw_fold = fold.with("raw");
    let mut open = has_details && ui.data(|d| d.get_temp::<bool>(fold)).unwrap_or(false);
    let mut raw = ui.data(|d| d.get_temp::<bool>(raw_fold)).unwrap_or(false);
    // Failures and results that need review get a tinted row and an edge bar.
    let mark = match status {
        pages::Status::Failed => Some(RED()),
        pages::Status::Review => Some(AMBER()),
        pages::Status::Done => None,
    };
    let background = ui.painter().add(Shape::Noop);
    let top = ui.cursor().top();
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 36.0), Sense::hover());
    let age = now.duration_since(activity.at).unwrap_or_default().as_secs();
    let unix = activity
        .at
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    let when = Rect::from_min_size(pos2(rect.left() + 16.0, rect.top()), vec2(64.0, rect.height()));
    ui.painter().text(
        when.left_center(),
        Align2::LEFT_CENTER,
        ago(age, unix),
        FontId::proportional(11.5),
        HINT(),
    );
    ui.interact(when, ui.scope_id().with("activity-when"), Sense::hover())
        .on_hover_text(format!("{} UTC", date(unix, 0)));
    pages::status_icon(
        ui.painter(),
        pos2(rect.left() + 96.0, rect.center().y),
        status,
    );
    let time = ui.painter().layout_no_wrap(
        format!("{:.1}s", activity.elapsed.as_secs_f32()),
        FontId::proportional(11.5),
        HINT(),
    );
    let mut right = rect.right() - 12.0;
    if has_details {
        let area = Rect::from_center_size(pos2(right - 13.0, rect.center().y), vec2(26.0, 26.0));
        let toggle = ui
            .interact(area, ui.scope_id().with("activity-toggle"), Sense::click())
            .on_hover_text(if open { tr("Hide details") } else { tr("Details") });
        toggle.widget_info(|| {
            WidgetInfo::selected(Role::Button, true, open, if open { tr("Hide details") } else { tr("Details") })
        });
        if toggle.hovered() || toggle.has_focus() {
            ui.painter().rect(
                area,
                6.0,
                HOVER(),
                Stroke::new(1.0, EDGE_HOT()),
                StrokeKind::Inside,
            );
        }
        icon(
            ui.painter(),
            Rect::from_center_size(area.center(), vec2(11.0, 11.0)),
            if open { Icon::Chevron } else { Icon::ChevronRight },
            if toggle.hovered() { TEXT() } else { MUTED() },
        );
        pages::control(
            widgets,
            format!("console-details-{index}"),
            &toggle,
            ui.clip_rect(),
        );
        if toggle.clicked() {
            open = !open;
            ui.data_mut(|d| d.insert_temp(fold, open));
        }
        right = area.left() - 6.0;
    } else {
        right -= 32.0;
    }
    let time_left = right - time.size().x;
    ui.painter().galley(
        pos2(time_left, rect.center().y - time.size().y / 2.0),
        time,
        HINT(),
    );
    // The operation, then its result in the remaining width.
    let text_left = rect.left() + 114.0;
    let limit = (time_left - 12.0 - text_left).max(0.0);
    let name = crate::operation_names::done(&activity.title);
    let mut job = text::LayoutJob::simple_singleline(name.clone(), semibold(12.5), TEXT());
    job.wrap = text::TextWrapping::truncate_at_width(limit.min(220.0));
    let title = ui.painter().layout_job(job);
    let title_width = title.size().x;
    ui.painter().galley(
        pos2(text_left, rect.center().y - title.size().y / 2.0),
        title,
        TEXT(),
    );
    let summary_left = text_left + title_width + 10.0;
    let mut job = text::LayoutJob::simple_singleline(
        outcome.summary.clone(),
        FontId::proportional(12.5),
        MUTED(),
    );
    job.wrap = text::TextWrapping::truncate_at_width((time_left - 12.0 - summary_left).max(0.0));
    let summary = ui.painter().layout_job(job);
    ui.painter().galley(
        pos2(summary_left, rect.center().y - summary.size().y / 2.0),
        summary,
        MUTED(),
    );
    ui.interact(
        Rect::from_min_max(pos2(text_left, rect.top()), pos2(time_left, rect.bottom())),
        ui.scope_id().with("activity-summary"),
        Sense::hover(),
    )
    .on_hover_text(format!("{name}\n{}", outcome.summary));
    if open {
        Frame::new()
            .inner_margin(Margin {
                left: 114,
                right: 16,
                top: 0,
                bottom: 12,
            })
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                Frame::new()
                    .fill(FOOTER())
                    .stroke(Stroke::new(1.0, HAIRLINE()))
                    .corner_radius(8)
                    .inner_margin(Margin::symmetric(12, 10))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.spacing_mut().item_spacing.y = 4.0;
                        for detail in &outcome.details {
                            ui.add(
                                Label::new(
                                    RichText::new(detail)
                                        .size(12.5)
                                        .color(TEXT_SOFT()),
                                )
                                .wrap(),
                            );
                        }
                        if let Some(path) = &outcome.retained_workspace {
                            ui.add(
                                Label::new(
                                    RichText::new(trf!("Temporary workspace: {path}", path = path.display()))
                                        .size(12.5)
                                        .color(MUTED()),
                                )
                                .selectable(true)
                                .wrap(),
                            );
                        }
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 4.0;
                            if !outcome.raw_output.is_empty() {
                                let label = if raw { tr("Hide Git output") } else { tr("Show Git output") };
                                let r = pages::tool(ui, Icon::Eye, label, true)
                                    .on_hover_text(tr("Local Git output; it may contain private repository data"));
                                pages::control(widgets, format!("console-output-{index}"), &r, ui.clip_rect());
                                if r.clicked() {
                                    raw = !raw;
                                    ui.data_mut(|d| d.insert_temp(raw_fold, raw));
                                }
                            }
                            let copy = pages::tool(ui, Icon::Copy, tr("Copy details"), true)
                                .on_hover_text(tr("Copies the summary and details, not Git output"));
                            pages::control(widgets, format!("console-copy-{index}"), &copy, ui.clip_rect());
                            if copy.clicked() {
                                let mut text = format!("{name}: {}", outcome.summary);
                                for detail in &outcome.details {
                                    text.push('\n');
                                    text.push_str(detail);
                                }
                                if let Some(path) = &outcome.retained_workspace {
                                    text.push_str(&format!("\n{}", trf!("Temporary workspace: {path}", path = path.display())));
                                }
                                ui.ctx().copy_text(text);
                            }
                        });
                        if raw && !outcome.raw_output.is_empty() {
                            ui.add_space(4.0);
                            ui.label(
                                RichText::new(tr("Local Git output. It may contain private repository data."))
                                    .size(12.0)
                                    .color(HINT()),
                            );
                            Frame::new()
                                .fill(DEEP())
                                .stroke(Stroke::new(1.0, HAIRLINE()))
                                .corner_radius(6)
                                .inner_margin(Margin::symmetric(10, 8))
                                .show(ui, |ui| {
                                    ui.set_width(ui.available_width());
                                    ScrollArea::vertical()
                                        .id_salt("activity-raw")
                                        .max_height(220.0)
                                        .show(ui, |ui| {
                                            ui.add(
                                                Label::new(
                                                    RichText::new(&outcome.raw_output)
                                                        .monospace()
                                                        .size(12.0),
                                                )
                                                .selectable(true)
                                                .wrap(),
                                            );
                                        });
                                });
                        }
                    });
            });
    }
    let bottom = ui.cursor().top();
    let whole = Rect::from_min_max(pos2(rect.left(), top), pos2(rect.right(), bottom));
    if let Some(color) = mark {
        ui.painter().set(
            background,
            Shape::rect_filled(whole, 0.0, color.gamma_multiply(0.05)),
        );
        ui.painter().rect_filled(
            Rect::from_min_size(whole.min, vec2(2.0, whole.height())),
            0.0,
            color,
        );
    }
    ui.painter().hline(
        rect.left() + 14.0..=rect.right() - 14.0,
        bottom,
        Stroke::new(1.0, HAIRLINE()),
    );
}

/// A start action on the welcome screen: icon tile, title, purpose and shortcut.
fn start_card(
    ui: &mut Ui,
    width: f32,
    image: Icon,
    title: &str,
    detail: &str,
    shortcut: &str,
    enabled: bool,
) -> Response {
    let response = ui.add_enabled(
        enabled,
        Button::new("").frame(false).min_size(vec2(width, 96.0)),
    );
    let rect = response.rect;
    let hot = enabled && (response.hovered() || response.has_focus());
    let painter = ui.painter();
    painter.rect(
        rect,
        10.0,
        if hot { HOVER() } else { PANEL() },
        Stroke::new(1.0, if hot { BORDER() } else { HAIRLINE() }),
        StrokeKind::Inside,
    );
    let ink = if enabled { TEXT() } else { FAINT() };
    let tile = Rect::from_min_size(rect.min + vec2(14.0, 14.0), vec2(28.0, 28.0));
    painter.rect_filled(tile, 7.0, if enabled { ACCENT_SOFT() } else { RAISED() });
    icon(
        painter,
        tile.shrink(7.0),
        image,
        if enabled { BLUE() } else { FAINT() },
    );
    painter.text(
        pos2(rect.left() + 14.0, rect.top() + 58.0),
        Align2::LEFT_CENTER,
        title,
        semibold(13.5),
        ink,
    );
    // The shortcut sits beside the icon so the purpose line keeps the full width.
    let chip = painter.layout_no_wrap(shortcut.into(), FontId::proportional(11.0), MUTED());
    let chip_rect = Rect::from_min_max(
        pos2(
            rect.right() - 12.0 - chip.size().x - 12.0,
            tile.center().y - 10.0,
        ),
        pos2(rect.right() - 12.0, tile.center().y + 10.0),
    );
    clipped_text(
        ui,
        Rect::from_min_max(
            pos2(rect.left() + 14.0, rect.bottom() - 30.0),
            pos2(rect.right() - 12.0, rect.bottom() - 12.0),
        ),
        detail,
        12.0,
        FAINT(),
    );
    let painter = ui.painter();
    painter.rect(
        chip_rect,
        5.0,
        RAISED(),
        Stroke::new(1.0, BORDER()),
        StrokeKind::Inside,
    );
    painter.galley(chip_rect.center() - chip.size() / 2.0, chip, MUTED());
    response.widget_info(|| WidgetInfo::labeled(Role::Button, enabled, title));
    response.on_hover_text(format!("{title} ({shortcut})"))
}

/// Display only: drop Windows' extended-length prefix (`\\?\`, `\\?\UNC\`).
/// Opening a recent repository still uses its exact stored path.
pub(crate) fn display_path(path: &std::path::Path) -> String {
    let lossy = path.to_string_lossy();
    match lossy.strip_prefix(r"\\?\") {
        Some(rest) => match rest.strip_prefix(r"UNC\") {
            Some(share) => format!(r"\\{share}"),
            None => rest.to_owned(),
        },
        None => lossy.into_owned(),
    }
}

/// A recent repository: initial tile, folder name and full path, and its branch once
/// HEAD has been read (`None` while reading, `Some(None)` when it could not be read).
fn recent_row(ui: &mut Ui, path: &std::path::Path, head: Option<Option<&Head>>) -> Response {
    let full = display_path(path);
    let name = path
        .file_name()
        .map_or_else(|| full.clone(), |n| n.to_string_lossy().into_owned());
    let response = ui.add(
        Button::new("")
            .frame(false)
            .min_size(vec2(ui.available_width(), 46.0)),
    );
    let rect = response.rect;
    style::row_background(
        ui.painter(),
        rect,
        false,
        response.hovered() || response.has_focus(),
    );
    let tile = Rect::from_center_size(pos2(rect.left() + 22.0, rect.center().y), vec2(28.0, 28.0));
    ui.painter().rect_filled(tile, 7.0, RAISED());
    ui.painter().text(
        tile.center(),
        Align2::CENTER_CENTER,
        name.chars()
            .next()
            .map_or_else(String::new, |c| c.to_uppercase().to_string()),
        semibold(12.5),
        BLUE(),
    );
    // The branch column has the same width on every row, read or not, so names and
    // paths line up and do not move when a result arrives.
    let column = (rect.width() * 0.28).min(140.0);
    let branch = Rect::from_min_max(
        pos2(rect.right() - 12.0 - column, rect.top()),
        pos2(rect.right() - 12.0, rect.bottom()),
    );
    let text = Rect::from_min_max(
        pos2(tile.right() + 12.0, rect.top()),
        pos2(branch.left() - 12.0, rect.bottom()),
    );
    clipped_text(
        ui,
        Rect::from_min_max(text.min, pos2(text.right(), rect.center().y + 1.0)),
        &name,
        13.0,
        TEXT(),
    );
    clipped_text(
        ui,
        Rect::from_min_max(pos2(text.left(), rect.center().y + 1.0), text.max),
        &full,
        11.5,
        FAINT(),
    );
    let mut hover = full;
    if let Some(head) = head {
        let label = head_name(head);
        if head.is_some() {
            let mark = Rect::from_center_size(
                pos2(branch.left() + 6.0, rect.center().y),
                vec2(12.0, 12.0),
            );
            icon(ui.painter(), mark, Icon::Branch, MUTED());
            clipped_text(
                ui,
                Rect::from_min_max(pos2(mark.right() + 6.0, branch.top()), branch.max),
                &label,
                12.5,
                MUTED(),
            );
        } else {
            clipped_text(ui, branch, &label, 12.0, FAINT());
        }
        hover = format!("{hover}\n{label}");
    }
    response.widget_info(|| WidgetInfo::labeled(Role::Button, true, &name));
    response.on_hover_text(hover)
}
pub(crate) fn commits(
    ui: &mut Ui,
    tab: &mut Tab,
    details: bool,
    row_height: f32,
    intents: &mut Vec<Intent>,
    widgets: &mut std::collections::HashMap<String, UiControl>,
    commands: &[rocket_git::CustomCommand],
) {
    let width = ui.available_width();
    let searching = !tab.search.trim().is_empty();
    let found = tab.commits().len();
    let state = &tab.search_state;
    let count = if searching && state.stopped {
        trf!("Stopped · {checked} commits checked", checked = pages::number_text(state.scanned))
    } else if searching && !state.complete && !state.scanning() && state.error.is_none() {
        trn!(found, "{found} commit · {checked} checked", "{found} commits · {checked} checked",
            found = pages::number_text(found), checked = pages::number_text(state.scanned))
    } else {
        trn!(found, "{found} commit", "{found} commits", found = pages::number_text(found))
    };
    let more = if searching {
        !tab.search_state.complete
    } else {
        tab.snapshot
            .as_ref()
            .and_then(|s| s.history.as_ref())
            .is_some_and(|h| h.next.is_some())
    };
    let bar = Frame::new()
        .fill(VIEW_PANEL())
        .inner_margin(Margin::symmetric(12, 7))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                let label = if tab.history_all {
                    tr("All branches").into()
                } else {
                    tab.history_ref
                        .as_ref()
                        .map_or_else(|| tr("Current branch").into(), |s| short_ref(s.as_bytes()))
                };
                let scope_width = (width * 0.24).clamp(150.0, 230.0);
                let selector = ui.add(Button::new("").frame(false).min_size(vec2(scope_width, 30.0)));
                widgets.insert("history-scope".into(), (&selector).into());
                let rect = selector.rect;
                ui.painter().rect(
                    rect,
                    7.0,
                    if selector.hovered() { HOVER() } else { RAISED() },
                    Stroke::new(1.0, BORDER()),
                    StrokeKind::Inside,
                );
                ui.painter().text(
                    pos2(rect.left() + 10.0, rect.center().y),
                    Align2::LEFT_CENTER,
                    tr("History"),
                    FontId::proportional(12.0),
                    FAINT(),
                );
                clipped_text(
                    ui,
                    Rect::from_min_max(
                        pos2(rect.left() + 60.0, rect.top()),
                        pos2(rect.right() - 24.0, rect.bottom()),
                    ),
                    &label,
                    13.0,
                    TEXT(),
                );
                icon(
                    ui.painter(),
                    Rect::from_center_size(pos2(rect.right() - 13.0, rect.center().y), vec2(10.0, 10.0)),
                    Icon::Chevron,
                    FAINT(),
                );
                selector.widget_info(|| {
                    WidgetInfo::labeled(Role::Button, true, trf!("History: {label}", label))
                });
                Popup::menu(&selector).show(|ui| {
                    let head = ui.button(tr("Current branch (HEAD)"));
                    widgets.insert("history-head".into(), (&head).into());
                    if head.clicked() {
                        intents.push(Intent::History(None));
                        ui.close();
                    }
                    let all = ui.button(tr("All branches"));
                    widgets.insert("history-all".into(), (&all).into());
                    if all.clicked() {
                        intents.push(Intent::AllBranches);
                        ui.close();
                    }
                    if let Some(snapshot) = &tab.snapshot {
                        ui.separator();
                        ui.label(RichText::new(tr("ONE BRANCH")).size(11.0).color(FAINT()));
                        for (i, branch) in snapshot.branches.iter().enumerate() {
                            if branch.symbolic_target.is_some() {
                                continue;
                            }
                            if let Ok(reference) = std::str::from_utf8(&branch.reference) {
                                let select = ui.button(short_ref(&branch.reference));
                                widgets.insert(format!("history-branch-{i}"), (&select).into());
                                if select.clicked() {
                                    intents.push(Intent::History(Some(reference.into())));
                                    ui.close();
                                }
                            }
                        }
                    }
                });
                let order = ui.button(order_label(tab.history_order))
                    .on_hover_text(tr("History order. Parents remain below their children in every order."));
                widgets.insert("history-order".into(), (&order).into());
                Popup::menu(&order).show(|ui| {
                    for value in [rocket_git::HistoryOrder::Topological, rocket_git::HistoryOrder::CommitDate, rocket_git::HistoryOrder::AuthorDate] {
                        let row = ui.selectable_label(tab.history_order == value, order_label(value));
                        widgets.insert(format!("history-order-{}", value.key()), (&row).into());
                        if row.clicked() { intents.push(Intent::HistoryOrder(value)); ui.close(); }
                    }
                    ui.label(RichText::new(tr("Parents follow their children.")).small().color(FAINT()));
                });
            });
            // Give scope/order their own row. In a wrapping row available_width
            // is the whole row, not the remaining space after preceding controls.
            let code = tab.search_kind == HistorySearchKind::Content;
            // In a narrow window Changed code puts the path and the status on a row of their own.
            let path_row = code && width < 1000.0;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                search_kind_selector(ui, tab, width, intents, widgets);
                let (hint, hover) = match tab.search_kind {
                    HistorySearchKind::Text => (
                        tr("Search messages, authors, IDs…"),
                        tr("Search messages, authors, email addresses or commit ID prefixes across the selected history."),
                    ),
                    HistorySearchKind::Path => (
                        tr("Search changed file paths…"),
                        tr("Find commits that changed a file whose path contains this text, ignoring case. Renames count as both paths; merges are not included."),
                    ),
                    HistorySearchKind::Content => (
                        tr("Search changed code"),
                        tr("Find commits that add or remove this text (git log -S), or with .* change a line matching it (git log -G). Reads file changes, so it checks 200 commits per step."),
                    ),
                };
                let search_width = if path_row {
                    ui.available_width()
                } else {
                    let reserve = if code { 260.0 + 208.0 } else { 260.0 };
                    (ui.available_width() - reserve).clamp(110.0, 440.0)
                };
                let toggles = code.then_some((&mut tab.search_case, &mut tab.search_regex));
                let (search, toggled) = query_field(
                    ui,
                    &mut tab.search,
                    hint,
                    search_width,
                    toggles,
                    tab.search_state.bad_pattern,
                    ["commit-filter", "search-case", "search-regex"],
                    widgets,
                );
                let search = search.on_hover_text(hover);
                if search.changed() || toggled {
                    intents.push(Intent::Search);
                }
                if code && !path_row && path_field(ui, &mut tab.search_path, 200.0, "commit-search-path", widgets) {
                    intents.push(Intent::Search);
                }
                if !path_row {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        search_status(ui, tab, searching, more, &count, intents, widgets);
                    });
                }
            });
            if path_row {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    if path_field(ui, &mut tab.search_path, 260.0, "commit-search-path", widgets) {
                        intents.push(Intent::Search);
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        search_status(ui, tab, searching, more, &count, intents, widgets);
                    });
                });
            }
        })
        .response
        .rect;
    ui.painter().hline(
        bar.x_range(),
        bar.bottom() - 0.5,
        Stroke::new(1.0, HAIRLINE()),
    );
    if let Some(error) = &tab.search_state.error {
        let color = if tab.search_state.bad_pattern { RED() } else { AMBER() };
        Frame::new()
            .fill(color.gamma_multiply(0.1))
            .inner_margin(Margin::symmetric(14, 6))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                let r = ui.horizontal(|ui| {
                    if tab.search_state.bad_pattern {
                        let (rect, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
                        icon(ui.painter(), rect, Icon::Warning, color);
                    }
                    ui.colored_label(color, error)
                });
                widgets.insert("history-search-error".into(), (&r.inner).into());
            });
    }
    crate::history_index::notice(ui, tab, intents, widgets);
    if details {
        // Preserve useful detail/diff space after a window shrinks; persisted panel
        // sizes must not push the lower pane beyond the viewport.
        let maximum = (ui.available_height() - 180.0).max(110.0);
        let height = (ui.available_height() * 0.48).clamp(110.0, maximum);
        let table = Panel::top(("history", tab.id))
            .default_size(height)
            .size_range(110.0..=maximum)
            .resizable(true)
            .frame(Frame::new().fill(VIEW_BG()))
            .show(ui, |ui| {
                history_table(ui, tab, row_height, intents, widgets, commands)
            })
            .response
            .rect;
        ui.painter().hline(
            table.x_range(),
            table.bottom() - 0.5,
            Stroke::new(1.0, HAIRLINE()),
        );
        CentralPanel::default()
            .frame(Frame::new().fill(VIEW_BG()))
            .show(ui, |ui| {
                crate::details::commit_details(ui, tab, widgets, intents, commands)
            });
    } else {
        history_table(ui, tab, row_height, intents, widgets, commands);
    }
    commit_peek(ui.ctx(), tab, widgets, intents);
}
/// The details popup Space opens over the commit list; Space, Escape or a click
/// elsewhere closes it.
fn commit_peek(
    ctx: &Context,
    tab: &mut Tab,
    widgets: &mut std::collections::HashMap<String, UiControl>,
    intents: &mut Vec<Intent>,
) {
    let Some(id) = &tab.peek else { return };
    let Some(commit) = tab.commits().iter().find(|c| &c.id == id).cloned() else {
        tab.peek = None;
        return;
    };
    let area = Area::new(Id::unique(("commit-peek", tab.id)))
        .order(Order::Foreground)
        .anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0))
        .show(ctx, |ui| {
            Frame::popup(ui.style())
                .fill(PANEL())
                .inner_margin(14)
                .show(ui, |ui| {
                    ui.set_width(520.0);
                    ScrollArea::vertical()
                        .id_salt(("commit-peek-scroll", tab.id))
                        .max_height(ctx.content_rect().height() * 0.6)
                        .show(ui, |ui| {
                            crate::details::commit_header(
                                ui,
                                &commit,
                                &mut tab.issue_links,
                                widgets,
                                intents,
                            );
                        });
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new(tr("Space or Esc closes this popup"))
                            .size(11.5)
                            .color(FAINT()),
                    );
                });
        });
    widgets.insert("commit-peek".into(), (&area.response).into());
    if area.response.clicked_elsewhere() || ctx.input(|i| i.key_pressed(Key::Escape)) {
        tab.peek = None;
    }
}
/// The stable id of a commit row, so keyboard focus can move to another row.
fn commit_row_id(tab: u64, commit: &ObjectId) -> Id {
    Id::unique(("commit-row", tab, commit.as_str()))
}
/// Keys of the focused commit row, read before any row takes input: Up and Down
/// select the neighbouring commit, Space toggles its details popup, Left and Right
/// collapse and expand a merge, and Tab or Shift+Tab leave the list for the pane
/// after or before it.
fn commit_list_keys(
    ui: &mut Ui,
    tab: &Tab,
    visible: std::ops::Range<usize>,
    row_height: f32,
    intents: &mut Vec<Intent>,
) {
    let commits = tab.commits();
    let Some(i) = visible
        .clone()
        .find(|&i| ui.memory(|m| m.has_focus(commit_row_id(tab.id, &commits[i].id))))
    else {
        return;
    };
    let focused = commit_row_id(tab.id, &commits[i].id);
    // Tab and the vertical arrows act on the list instead of moving focus row by row.
    ui.memory_mut(|m| {
        m.set_focus_lock_filter(
            focused,
            EventFilter {
                tab: true,
                vertical_arrows: true,
                horizontal_arrows: true,
                escape: false,
            },
        )
    });
    let pressed = |modifiers, key| ui.input_mut(|input| input.consume_key(modifiers, key));
    if pressed(Modifiers::NONE, Key::Space) {
        intents.push(Intent::PeekCommit(commits[i].id.clone()));
    }
    let (left, right) = (
        pressed(Modifiers::NONE, Key::ArrowLeft),
        pressed(Modifiers::NONE, Key::ArrowRight),
    );
    if commits[i].parents.len() > 1 && tab.search.trim().is_empty() {
        let collapsed = tab.folding.collapsed(&commits[i]);
        if (left && !collapsed) || (right && collapsed) {
            intents.push(Intent::FoldMerge(commits[i].id.clone(), left));
        }
    }
    let (down, up) = (
        pressed(Modifiers::NONE, Key::ArrowDown),
        pressed(Modifiers::NONE, Key::ArrowUp),
    );
    if down || up {
        // egui would also move focus to the nearest widget; the list moves it instead,
        // and keeps it at the first or last commit.
        ui.memory_mut(|m| m.move_focus(FocusDirection::None));
    }
    let next = if down {
        (i + 1 < commits.len()).then_some(i + 1)
    } else if up {
        i.checked_sub(1)
    } else {
        None
    };
    let (tab_out, back) = (
        pressed(Modifiers::NONE, Key::Tab),
        pressed(Modifiers::SHIFT, Key::Tab),
    );
    if let Some(next) = next {
        let target = &commits[next].id;
        intents.push(Intent::SelectCommit(target.clone()));
        ui.memory_mut(|m| m.request_focus(commit_row_id(tab.id, target)));
        let top = ui.cursor().top() + (next as f32 - visible.start as f32) * row_height;
        ui.scroll_to_rect_animation(
            Rect::from_min_size(
                pos2(ui.cursor().left(), top),
                vec2(ui.available_width(), row_height),
            ),
            None,
            eframe::egui::style::ScrollAnimation::none(),
        );
    } else if tab_out || back {
        // Hand focus to the last (or first) visible row with egui's own traversal
        // pending, so the widget after (or before) the list receives it.
        let edge = if tab_out {
            visible.end - 1
        } else {
            visible.start
        };
        ui.memory_mut(|m| {
            m.request_focus(commit_row_id(tab.id, &commits[edge].id));
            m.move_focus(if tab_out {
                FocusDirection::Next
            } else {
                FocusDirection::Previous
            });
        });
    }
}
/// What an empty commit list says during or after a search, and a hint below it.
fn empty_search(tab: &Tab) -> (String, Option<&'static str>) {
    let state = &tab.search_state;
    if state.bad_pattern {
        return (tr("Fix the expression, or turn off .* to search for the text as written.").into(), None);
    }
    if state.refused {
        return (tr("Nothing was searched. Fix the path, or clear it to search everywhere.").into(), None);
    }
    if state.scanning() {
        return (tr("Still searching older commits.").into(), None);
    }
    if state.stopped {
        return (
            trf!("Nothing found in the {checked} commits checked.", checked = pages::number_text(state.scanned)),
            Some(tr("Resume continues with older commits.")),
        );
    }
    if !state.complete {
        return (tr("No matches yet. The history search is not complete.").into(), None);
    }
    if state.kind != HistorySearchKind::Content {
        return (tr("No commits match this search in the selected history.").into(), None);
    }
    let scope = if tab.history_all {
        tr("all branches").to_owned()
    } else {
        tab.history_ref
            .as_ref()
            .map_or_else(|| tr("the current branch").into(), |r| short_ref(r.as_bytes()))
    };
    let under = state
        .content
        .path
        .as_ref()
        .map(|p| crate::tree::path_label(p.as_bytes()));
    if state.content.regex {
        let text = match &under {
            Some(path) => trf!("No commits in {scope} change a line matching this under {path}.", scope, path),
            None => trf!("No commits in {scope} change a line matching this.", scope),
        };
        (text, None)
    } else {
        (
            match &under {
                Some(path) => trf!("No commits in {scope} add or remove this text under {path}.", scope, path),
                None => trf!("No commits in {scope} add or remove this text.", scope),
            },
            Some(tr("Exact text finds a change in how often it appears. Turn on .* to include edited lines.")),
        )
    }
}

/// The commit search's status at the right of its bar: Browse files, then Stop while
/// it runs, or Resume or Load more and the count.
fn search_status(
    ui: &mut Ui,
    tab: &mut Tab,
    searching: bool,
    more: bool,
    count: &str,
    intents: &mut Vec<Intent>,
    widgets: &mut std::collections::HashMap<String, UiControl>,
) {
    ui.spacing_mut().item_spacing.x = 6.0;
    let browse = ui
        .add_enabled(
            tab.selected_commit.is_some(),
            quiet(Button::new(RichText::new(tr("Browse files")).size(12.5))),
        )
        .on_hover_text(tr("Browse the repository files at the selected commit"));
    widgets.insert("commit-browse-files".into(), (&browse).into());
    if browse.clicked() {
        intents.push(Intent::Tree(crate::tree::TreeIntent::OpenSelected));
    }
    if searching && tab.search_state.scanning() {
        let stop = ui.button(tr("Stop"));
        widgets.insert("history-stop".into(), (&stop).into());
        if stop.clicked() {
            intents.push(Intent::StopSearch);
        }
        ui.label(
            RichText::new(trf!("{checked} commits checked", checked = pages::number_text(tab.search_state.scanned)))
                .size(12.0)
                .color(FAINT()),
        );
        ui.spinner();
        return;
    }
    if more && !tab.search_state.bad_pattern && !tab.search_state.refused {
        let resume = searching && tab.search_state.paused;
        let load = ui.add_enabled(
            tab.more.is_none() && !tab.loading(),
            Button::new(RichText::new(if resume { tr("Resume") } else { tr("Load more") }).size(12.5)),
        );
        widgets.insert("history-more".into(), (&load).into());
        if load.clicked() {
            intents.push(Intent::More);
        }
    }
    ui.label(RichText::new(count).size(12.0).color(FAINT()))
        .on_hover_text(if searching {
            format!(
                "{} · {}",
                trf!("{checked} commits checked", checked = pages::number_text(tab.search_state.scanned)),
                if tab.search_state.complete { tr("complete") } else { tr("more history remains") }
            )
        } else {
            trf!("{order} order: parents follow their children. All branches combines local and remote-tracking branches, plus HEAD.", order = order_label(tab.history_order))
        });
}

/// One toggle inside a search field, such as Aa or .*.
fn field_toggle(ui: &mut Ui, label: &str, on: bool, hover: &str) -> Response {
    let ink = if on { BLUE() } else { FAINT() };
    let text = if label == ".*" {
        RichText::new(label).monospace().size(12.0).color(ink)
    } else {
        RichText::new(label).strong().size(12.0).color(ink)
    };
    ui.add(
        Button::new(text)
            .selected(on)
            .fill(if on { BLUE().gamma_multiply(0.16) } else { Color32::TRANSPARENT })
            .stroke(Stroke::new(1.0, if on { BLUE().gamma_multiply(0.45) } else { Color32::TRANSPARENT }))
            .corner_radius(5)
            .min_size(vec2(26.0, 22.0)),
    )
    .on_hover_text(hover)
}

/// A search field with an optional Match case and Regular expression toggle inside it,
/// outlined red while Git refuses the expression. `ids` names the field and toggles for
/// MCP. Returns the field and whether a toggle changed.
#[allow(clippy::too_many_arguments)]
pub(crate) fn query_field(
    ui: &mut Ui,
    value: &mut String,
    hint: &str,
    width: f32,
    toggles: Option<(&mut bool, &mut bool)>,
    bad: bool,
    ids: [&str; 3],
    widgets: &mut std::collections::HashMap<String, UiControl>,
) -> (Response, bool) {
    let mut toggled = false;
    // 22 points of text in a 3-point margin and a 1-point border: 30 points, the
    // height of the search-kind and scope pickers beside it.
    let field = Frame::new()
        .fill(RAISED())
        .stroke(Stroke::new(1.0, if bad { RED() } else { BORDER() }))
        .corner_radius(6)
        .inner_margin(Margin { left: 10, right: 4, top: 3, bottom: 3 })
        .show(ui, |ui| {
            ui.set_width((width - 16.0).max(30.0));
            ui.spacing_mut().interact_size.y = 22.0;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                // Keeps the Aa and .* toggles 22 points tall, the text line's height:
                // taller ones made the field 34 points and pushed the row out of line.
                ui.spacing_mut().button_padding.y = 2.0;
                let (rect, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
                icon(ui.painter(), rect, Icon::Search, MUTED());
                let room = if toggles.is_some() { 60.0 } else { 0.0 };
                let edit = ui.add(
                    TextEdit::singleline(value)
                        .frame(Frame::NONE)
                        .char_limit(1024)
                        .min_size(vec2(0.0, 22.0))
                        .align(Align2::LEFT_CENTER)
                        .hint_text(hint)
                        .desired_width((ui.available_width() - room).max(20.0)),
                );
                if let Some((case, regex)) = toggles {
                    let r = field_toggle(ui, "Aa", *case, tr("Match case"));
                    widgets.insert(ids[1].into(), (&r).into());
                    if r.clicked() {
                        *case = !*case;
                        toggled = true;
                    }
                    let r = field_toggle(ui, ".*", *regex, tr("Regular expression, as git log -G and git grep -E read it"));
                    widgets.insert(ids[2].into(), (&r).into());
                    if r.clicked() {
                        *regex = !*regex;
                        toggled = true;
                    }
                }
                edit
            })
            .inner
        })
        .inner;
    widgets.insert(ids[0].into(), (&field).into());
    (field, toggled)
}

/// A folder-limit field with a clear button. Returns whether the path changed.
pub(crate) fn path_field(
    ui: &mut Ui,
    value: &mut String,
    width: f32,
    id: &str,
    widgets: &mut std::collections::HashMap<String, UiControl>,
) -> bool {
    let mut changed = false;
    // The same 30 points as the query field it sits beside.
    Frame::new()
        .fill(RAISED())
        .stroke(Stroke::new(1.0, BORDER()))
        .corner_radius(6)
        .inner_margin(Margin { left: 10, right: 4, top: 3, bottom: 3 })
        .show(ui, |ui| {
            ui.set_width((width - 16.0).max(30.0));
            ui.spacing_mut().interact_size.y = 22.0;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                let (rect, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
                icon(ui.painter(), rect, Icon::Folder, MUTED());
                let room = if value.is_empty() { 0.0 } else { 26.0 };
                let edit = ui
                    .add(
                        TextEdit::singleline(value)
                            .frame(Frame::NONE)
                            .char_limit(4096)
                            .min_size(vec2(0.0, 22.0))
                            .align(Align2::LEFT_CENTER)
                            .hint_text(tr("In path"))
                            .desired_width((ui.available_width() - room).max(20.0)),
                    )
                    .on_hover_text(tr("Only this file or folder, as written in the repository: case matters, \\ reads as /"));
                widgets.insert(id.into(), (&edit).into());
                changed |= edit.changed();
                if !value.is_empty() {
                    let clear = crate::chrome::icon_button(ui, Icon::Close, true, false, 22.0)
                        .on_hover_text(tr("Clear the path"));
                    widgets.insert(format!("{id}-clear"), (&clear).into());
                    if clear.clicked() {
                        value.clear();
                        changed = true;
                    }
                }
            });
        });
    changed
}

/// Chooses what the commit search compares its text with. Changing it restarts a
/// search that has text.
fn search_kind_selector(
    ui: &mut Ui,
    tab: &mut Tab,
    width: f32,
    intents: &mut Vec<Intent>,
    widgets: &mut std::collections::HashMap<String, UiControl>,
) {
    let label = match tab.search_kind {
        HistorySearchKind::Text => tr("Text"),
        HistorySearchKind::Path => tr("Path"),
        HistorySearchKind::Content => tr("Code"),
    };
    let selector_width = if width < 900.0 { 86.0 } else { 100.0 };
    let selector = ui.add(
        Button::new("")
            .frame(false)
            .min_size(vec2(selector_width, 30.0)),
    );
    widgets.insert("search-kind".into(), (&selector).into());
    let rect = selector.rect;
    ui.painter().rect(
        rect,
        7.0,
        if selector.hovered() { HOVER() } else { RAISED() },
        Stroke::new(1.0, BORDER()),
        StrokeKind::Inside,
    );
    clipped_text(
        ui,
        Rect::from_min_max(
            pos2(rect.left() + 10.0, rect.top()),
            pos2(rect.right() - 22.0, rect.bottom()),
        ),
        label,
        13.0,
        TEXT(),
    );
    icon(
        ui.painter(),
        Rect::from_center_size(pos2(rect.right() - 12.0, rect.center().y), vec2(10.0, 10.0)),
        Icon::Chevron,
        FAINT(),
    );
    selector.widget_info(|| {
        WidgetInfo::labeled(Role::Button, true, trf!("Search in: {label}", label))
    });
    let selector = selector.on_hover_text(tr("Choose what the commit search looks at"));
    Popup::menu(&selector).show(|ui| {
        for (kind, key, text, detail) in [
            (
                HistorySearchKind::Text,
                "search-kind-text",
                tr("Messages, authors and IDs"),
                "",
            ),
            (
                HistorySearchKind::Path,
                "search-kind-path",
                tr("Changed file paths"),
                "",
            ),
            (
                HistorySearchKind::Content,
                "search-kind-content",
                tr("Changed code"),
                "   git log -S / -G",
            ),
        ] {
            let mut job = text::LayoutJob::default();
            job.append(text, 0.0, TextFormat::simple(FontId::proportional(13.0), TEXT()));
            job.append(detail, 0.0, TextFormat::simple(FontId::monospace(11.5), FAINT()));
            // A preview lacks most file versions, which Changed code reads.
            let enabled = !(kind == HistorySearchKind::Content && tab.is_preview());
            let choice = ui
                .add_enabled(enabled, Button::selectable(tab.search_kind == kind, job))
                .on_disabled_hover_text(tr("Needs every file version: keep this preview to search changed code"));
            widgets.insert(key.into(), (&choice).into());
            if choice.clicked() {
                if tab.search_kind != kind {
                    tab.search_kind = kind;
                    if !tab.search.trim().is_empty() {
                        intents.push(Intent::Search);
                    }
                }
                ui.close();
            }
        }
    });
}

/// One label in the commit list: a branch, or in the compact style a branch together
/// with the remote branch it tracks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Chip {
    label: String,
    /// What hovering a compact chip names in full.
    full: String,
    remote: bool,
    head: bool,
    tracked: bool,
    /// The count of branches that didn't fit.
    more: bool,
}
impl Chip {
    fn more(count: usize) -> Self {
        Self { label: format!("+{count}"), full: String::new(), remote: true, head: false, tracked: false, more: true }
    }
    /// Draws the chip at `x`, unless it would pass `limit`.
    fn paint(&self, painter: &Painter, lane: Color32, x: f32, middle: f32, limit: f32) -> Option<Rect> {
        let (fill, text) = if self.head {
            (BLUE(), ACCENT_INK())
        } else if self.remote {
            (Color32::TRANSPARENT, MUTED())
        } else {
            (lane.gamma_multiply(0.16), lane)
        };
        let font = if self.remote { FontId::proportional(11.5) } else { semibold(11.5) };
        let galley = painter.layout_no_wrap(self.label.clone(), font, text);
        let size = vec2(galley.size().x + 26.0 + if self.tracked { 14.0 } else { 0.0 }, 20.0);
        if x + size.x > limit {
            return None;
        }
        let pill = Rect::from_center_size(pos2(x + size.x / 2.0, middle), size);
        painter.rect_filled(pill, 5.0, fill);
        if self.remote {
            painter.rect_stroke(pill, 5.0, Stroke::new(1.0, BORDER()), StrokeKind::Inside);
        }
        let mark = |at: f32, image: Icon| {
            icon(painter, Rect::from_center_size(pos2(at, pill.center().y), vec2(11.0, 11.0)), image, text);
        };
        mark(pill.left() + 11.0, if self.remote && !self.more { Icon::Remote } else { Icon::Branch });
        if self.tracked {
            mark(pill.right() - 11.0, Icon::Remote);
        }
        painter.galley(pos2(pill.left() + 20.0, pill.center().y - galley.size().y / 2.0), galley, text);
        Some(pill)
    }
}

/// rocket-ui-test/commands reads r/commands: each folder keeps its first letter.
fn short_folders(name: &str) -> String {
    let mut parts: Vec<String> = name.split('/').map(str::to_owned).collect();
    let last = parts.len() - 1;
    for part in &mut parts[..last] {
        *part = part.chars().take(1).collect();
    }
    parts.join("/")
}

/// A branch's compact name. The remote's name goes when there is only one, because the
/// chip's cloud already says remote.
fn compact_name(branch: &Branch, remotes: &[Remote]) -> String {
    let name = short_ref(&branch.reference);
    if branch.kind != BranchKind::Remote {
        return short_folders(&name);
    }
    let owner = remotes.iter().map(|r| display(&r.name)).find(|r| name.starts_with(&format!("{r}/")));
    match owner {
        Some(remote) if remotes.len() == 1 => short_folders(&name[remote.len() + 1..]),
        Some(remote) => format!("{remote}/{}", short_folders(&name[remote.len() + 1..])),
        None => short_folders(&name),
    }
}

/// The branches on each of `commits`, in the order the repository lists them, found in
/// one pass over `branches`. Symbolic refs such as origin/HEAD are left out: they get
/// no chip.
pub(crate) fn branch_tips<'a>(
    commits: &'a [Commit],
    branches: &'a [Branch],
) -> std::collections::HashMap<&'a ObjectId, Vec<&'a Branch>> {
    let mut tips: std::collections::HashMap<&ObjectId, Vec<&Branch>> =
        commits.iter().map(|commit| (&commit.id, Vec::new())).collect();
    for branch in branches.iter().filter(|b| b.symbolic_target.is_none()) {
        if let Some(here) = tips.get_mut(&branch.target) {
            here.push(branch);
        }
    }
    tips
}

/// The chips for the branches on one commit. Compact folds a branch and the remote branch
/// it tracks into one chip and shortens the names.
pub(crate) fn branch_chips(branches: &[&Branch], remotes: &[Remote], compact: bool) -> Vec<Chip> {
    let chip = |branch: &Branch, label: String, full: String, tracked: bool| Chip {
        label,
        full,
        remote: branch.kind == BranchKind::Remote,
        head: branch.is_head,
        tracked,
        more: false,
    };
    if !compact {
        return branches.iter().map(|b| chip(b, short_ref(&b.reference), short_ref(&b.reference), false)).collect();
    }
    fn tracked_here<'a>(local: &'a Branch, branches: &[&Branch]) -> Option<&'a Vec<u8>> {
        local.upstream.as_ref().filter(|u| branches.iter().any(|b| b.reference == **u))
    }
    branches
        .iter()
        .filter(|b| !branches.iter().any(|l| tracked_here(l, branches) == Some(&b.reference)))
        .map(|b| {
            let name = short_ref(&b.reference);
            let tracked = tracked_here(b, branches);
            let full = tracked.map_or_else(|| name.clone(), |u| format!("{name} · {}", short_ref(u)));
            chip(b, compact_name(b, remotes), full, tracked.is_some())
        })
        .collect()
}

/// Preferences → General: sample rows drawn by the commit list's own chips.
pub(crate) fn label_preview(ui: &mut Ui, compact: bool) {
    const ROW: f32 = 33.0;
    let sample = |label: &str, remote: bool, head: bool, tracked: bool| Chip {
        label: label.to_owned(),
        full: String::new(),
        remote,
        head,
        tracked,
        more: false,
    };
    let rows = [
        ("Add folder browser to the Open dialog", "4f1c2ab", if compact {
            vec![sample("main", false, true, true)]
        } else {
            vec![sample("main", false, true, false), sample("origin/main", true, false, false)]
        }),
        ("Group diagnostics by session", "c71d5b0", vec![if compact {
            sample("r/commands", true, false, false)
        } else {
            sample("origin/rocket-ui-test/commands", true, false, false)
        }]),
        ("Move MCP settings into Preferences", "2be94f3", if compact {
            vec![sample("r/shortcut-keys", false, false, true)]
        } else {
            vec![sample("rocket-ui-test/shortcut-keys", false, false, false), sample("origin/rocket-ui-test/shortcut-keys", true, false, false)]
        }),
    ];
    let height = ROW * rows.len() as f32 + 2.0;
    let (outer, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    let painter = ui.painter_at(outer);
    painter.rect_filled(outer, 4.0, BG());
    let rail = outer.left() + 18.0;
    painter.vline(rail, outer.y_range(), Stroke::new(1.0, BLUE()));
    for (i, (subject, id, chips)) in rows.iter().enumerate() {
        let middle = outer.top() + 1.0 + ROW * (i as f32 + 0.5);
        painter.circle(pos2(rail, middle), 4.0, BG(), Stroke::new(2.0, BLUE()));
        let mut x = outer.left() + 34.0;
        for chip in chips {
            if let Some(pill) = chip.paint(&painter, ORANGE(), x, middle, outer.right() - 140.0) {
                x = pill.right() + 6.0;
            }
        }
        let code = painter.layout_no_wrap((*id).to_owned(), FontId::monospace(12.0), HINT());
        let id_left = outer.right() - 12.0 - code.size().x;
        painter.galley(pos2(id_left, middle - code.size().y / 2.0), code, HINT());
        let mut job = text::LayoutJob::simple_singleline((*subject).to_owned(), FontId::proportional(13.5), TEXT());
        job.wrap = text::TextWrapping::truncate_at_width((id_left - x - 10.0).max(0.0));
        let galley = painter.layout_job(job);
        painter.galley(pos2(x + 2.0, middle - galley.size().y / 2.0), galley, TEXT());
    }
    painter.rect_stroke(outer, 4.0, Stroke::new(1.0, HAIRLINE()), StrokeKind::Inside);
}

fn history_table(
    ui: &mut Ui,
    tab: &Tab,
    row_height: f32,
    intents: &mut Vec<Intent>,
    widgets: &mut std::collections::HashMap<String, UiControl>,
    commands: &[rocket_git::CustomCommand],
) {
    ui.set_min_size(ui.available_size());
    let compact = crate::general::options(ui.ctx()).compact_labels;
    if tab.snapshot.as_ref().is_some_and(|s| s.history.is_none()) {
        Frame::new().inner_margin(14).show(ui, |ui| {
            ui.colored_label(AMBER(), tr("History is unavailable. Review the read error above, then refresh or choose another branch."));
        });
        return;
    }
    if tab.graph.iter().any(|row| row.limited) {
        ui.colored_label(AMBER(), tr("Graph simplified: some ancestry exceeds the display limit."));
    }
    let graph_width = (tab.graph.iter().map(|r| r.lane).max().unwrap_or(0) as f32 * 16.0 + 34.0)
        .clamp(72.0, 190.0);
    let width = ui.available_width();
    let show_author = width > 1000.0;
    let sha_width = if width > 1150.0 { 84.0 } else { 0.0 };
    let date_width = if width > 780.0 { 132.0 } else { 0.0 };
    let author_width = if show_author { 180.0 } else { 0.0 };
    let message_end = (width - author_width - date_width - sha_width).max(graph_width + 120.0);
    let previous_spacing = ui.spacing().item_spacing.y;
    ui.spacing_mut().item_spacing.y = 0.0;
    let header = ui.allocate_exact_size(vec2(width, 28.0), Sense::hover()).0;
    ui.painter().hline(
        header.x_range(),
        header.bottom() - 0.5,
        Stroke::new(1.0, HAIRLINE()),
    );
    for (x, label) in [
        (14.0, tr("GRAPH")),
        (graph_width + 2.0, tr("MESSAGE")),
        (message_end + 8.0, if show_author { tr("AUTHOR") } else { "" }),
        (
            width - date_width - sha_width + 8.0,
            if date_width > 0.0 { tr("DATE") } else { "" },
        ),
        (
            width - sha_width + 8.0,
            if sha_width > 0.0 { "SHA" } else { "" },
        ),
    ] {
        ui.painter().text(
            header.left_top() + vec2(x, 14.0),
            Align2::LEFT_CENTER,
            label,
            semibold(10.5),
            FAINT(),
        );
    }
    let query = tab.search.trim();
    let count = tab.commits().len();
    if count == 0 {
        ui.spacing_mut().item_spacing.y = previous_spacing;
        Frame::new().inner_margin(16).show(ui, |ui| {
            if query.is_empty() {
                ui.label(
                    RichText::new(tr("No commits yet. Stage files in Changes and create your first commit."))
                        .color(MUTED()),
                );
                return;
            }
            let (title, hint) = empty_search(tab);
            let r = ui.label(RichText::new(title).color(if hint.is_some() { TEXT() } else { MUTED() }));
            widgets.insert("history-empty".into(), (&r).into());
            if let Some(hint) = hint {
                ui.label(RichText::new(hint).color(MUTED()));
            }
        });
        return;
    }
    let output = ScrollArea::vertical()
        .id_salt(("history-scroll", tab.id))
        .auto_shrink([false, false])
        .show_rows(ui, row_height, count, |ui, range| {
            ui.spacing_mut().item_spacing.y = 0.0;
            commit_list_keys(ui, tab, range.clone(), row_height, intents);
            // One pass over the branches for the rows shown, not one for each row: a
            // repository can have thousands of branches.
            let tips = tab
                .snapshot
                .as_ref()
                .map(|snapshot| branch_tips(&tab.commits()[range.clone()], &snapshot.branches));
            for i in range {
                let index = i;
                let commit = &tab.commits()[index];
                let (rect, _) =
                    ui.allocate_exact_size(vec2(ui.available_width(), row_height), Sense::hover());
                let response = ui.interact(rect, commit_row_id(tab.id, &commit.id), Sense::click());
                let response = crate::drag::commit(ui, response, tab, commit, intents);
                // Virtual lists also render overscan rows. Automation must use
                // the visible part, never a center hidden behind the details pane.
                crate::pages::control(widgets, format!("commit-{}", commit.id), &response, ui.clip_rect());
                response.context_menu(|ui| {
                    if query.is_empty() { fold_menu(ui, tab, commit, intents, widgets); }
                    let mut context = crate::custom_commands::context(tab);
                    context.commit = Some(commit.id.clone());
                    crate::custom_commands::context_menu(ui, commands, rocket_git::CommandScope::Commit, &context, intents, widgets);
                    let target = crate::integrations::CommandTarget::commit(&tab.path, &commit.id);
                    crate::integrations::context_rows(ui, &target, crate::integrations::Divider::Before, intents, widgets);
                });
                let primary = tab.selected_commit.as_ref() == Some(&commit.id);
                // Ctrl/Shift-click rows share the selection fill; the accent bar
                // marks the commit shown in the details pane.
                let selected = primary || tab.selected_commits.contains(&commit.id);
                if selected {
                    ui.painter().rect_filled(rect, 0.0, SELECTED());
                    if primary {
                        ui.painter().rect_filled(
                            Rect::from_min_size(rect.min, vec2(3.0, rect.height())),
                            0.0,
                            BLUE(),
                        );
                    }
                } else if response.hovered() || response.has_focus() {
                    ui.painter().rect_filled(rect, 0.0, HOVER());
                }
                let painter = ui.painter().with_clip_rect(rect);
                let node_ring = if selected { SELECTED() } else { BG() };
                if !query.is_empty() {
                    painter.circle_filled(pos2(rect.left() + 22.0, rect.center().y), 4.5, BLUE());
                } else if let Some(row) = tab.graph.get(index) {
                    let x = |lane| rect.left() + 18.0 + lane as f32 * 16.0;
                    for edge in &row.edges {
                        let from = pos2(
                            x(edge.from),
                            if edge.from_node {
                                rect.center().y
                            } else {
                                rect.top()
                            },
                        );
                        let to = pos2(
                            x(edge.to),
                            if edge.to_node {
                                rect.center().y
                            } else {
                                rect.bottom()
                            },
                        );
                        let graph_painter = painter.with_clip_rect(Rect::from_min_max(
                            rect.min,
                            pos2(rect.left() + graph_width, rect.bottom()),
                        ));
                        let middle = (from.y + to.y) * 0.5;
                        graph_painter.add(epaint::CubicBezierShape::from_points_stroke(
                            [from, pos2(from.x, middle), pos2(to.x, middle), to],
                            false,
                            Color32::TRANSPARENT,
                            Stroke::new(2.0, LANES()[edge.color % LANES().len()]),
                        ));
                    }
                    let center = pos2(
                        x(row.lane).min(rect.left() + graph_width - 8.0),
                        rect.center().y,
                    );
                    let color = LANES()[row.color % LANES().len()];
                    if tab.folding.collapsed(commit) {
                        // A collapsed merge: a larger ring with a plus.
                        painter.circle(center, 5.6, node_ring, Stroke::new(2.0, color));
                        let stroke = Stroke::new(1.6, color);
                        painter.line_segment([center - vec2(2.6, 0.0), center + vec2(2.6, 0.0)], stroke);
                        painter.line_segment([center - vec2(0.0, 2.6), center + vec2(0.0, 2.6)], stroke);
                    } else if commit.parents.len() > 1 {
                        painter.circle(center, 4.0, node_ring, Stroke::new(2.0, color));
                    } else {
                        painter.circle(center, 4.8, color, Stroke::new(2.0, node_ring));
                    }
                }
                let mut x = rect.left() + graph_width;
                // A matched hosting repository keeps a slot for the CI mark at the
                // message's end, so marks line up and arriving ones move nothing.
                let mark_room = if tab.ci.active() { 26.0 } else { 0.0 };
                let message_clip = Rect::from_min_max(
                    pos2(x, rect.top()),
                    pos2(rect.left() + message_end - 8.0 - mark_room, rect.bottom()),
                );
                let message = painter.with_clip_rect(message_clip);
                if let Some(snapshot) = &tab.snapshot {
                    let here = tips
                        .as_ref()
                        .and_then(|tips| tips.get(&commit.id))
                        .map_or(&[][..], Vec::as_slice);
                    let chips = branch_chips(here, &snapshot.remotes, compact);
                    let lane = LANES()[tab.graph.get(index).map_or(1, |r| r.color) % LANES().len()];
                    let limit = message_clip.right() - 70.0;
                    let chip_id = |i: usize| Id::unique(("commit-chip", tab.id, commit.id.as_str(), i));
                    let mut shown = 0;
                    for (i, chip) in chips.iter().enumerate().take(3) {
                        // Compact keeps room to count what doesn't fit.
                        let room = limit - if compact && i + 1 < chips.len() { 46.0 } else { 0.0 };
                        let Some(pill) = chip.paint(&message, lane, x, rect.center().y, room) else {
                            break;
                        };
                        if compact {
                            ui.interact(pill, chip_id(i), Sense::hover()).on_hover_text(&chip.full);
                        }
                        x = pill.right() + 6.0;
                        shown += 1;
                    }
                    if compact && shown < chips.len() {
                        let more = Chip::more(chips.len() - shown);
                        if let Some(pill) = more.paint(&message, lane, x, rect.center().y, limit) {
                            let names: Vec<&str> = chips[shown..].iter().map(|c| c.full.as_str()).collect();
                            ui.interact(pill, chip_id(chips.len()), Sense::hover()).on_hover_text(names.join("\n"));
                            x = pill.right() + 6.0;
                        }
                    }
                }
                if query.is_empty()
                    && let Some(hidden) = tab.behind.get(&commit.id)
                {
                    // A collapsed merge: how many commits it hides; a click shows them.
                    let galley = message.layout_no_wrap(format!("+{hidden}"), semibold(11.5), MUTED());
                    let size = vec2(galley.size().x + 16.0, 20.0);
                    let pill = Rect::from_center_size(pos2(x + size.x / 2.0, rect.center().y), size);
                    let expand = ui.interact(
                        pill,
                        Id::unique(("commit-expand", tab.id, commit.id.as_str())),
                        Sense::click(),
                    );
                    message.rect_filled(pill, 10.0, if expand.hovered() { HOVER() } else { RAISED() });
                    message.rect_stroke(pill, 10.0, Stroke::new(1.0, BORDER()), StrokeKind::Inside);
                    message.galley(
                        pos2(pill.left() + 8.0, pill.center().y - galley.size().y / 2.0),
                        galley,
                        MUTED(),
                    );
                    crate::pages::control(widgets, format!("commit-expand-{}", commit.id), &expand, ui.clip_rect());
                    if expand.clicked() {
                        intents.push(Intent::FoldMerge(commit.id.clone(), false));
                    }
                    expand.on_hover_text(trf!(
                        "{hidden} commits of this merge are hidden. Click, or press → on the row, to show them.",
                        hidden
                    ));
                    x += size.x + 6.0;
                }
                let color = if commit.parents.len() > 1 {
                    MUTED()
                } else {
                    TEXT()
                };
                // End long subjects with an ellipsis rather than cutting them at the clip.
                let mut job = text::LayoutJob::simple_singleline(
                    subject(commit),
                    FontId::proportional(13.5),
                    color,
                );
                job.wrap = text::TextWrapping::truncate_at_width(
                    (message_clip.right() - x - 2.0).max(0.0),
                );
                let galley = message.layout_job(job);
                message.galley(
                    pos2(x + 2.0, rect.center().y - galley.size().y / 2.0),
                    galley,
                    color,
                );
                if let Some(state) = tab.ci.mark(commit.id.as_str()) {
                    let center = pos2(rect.left() + message_end - 8.0 - 13.0, rect.center().y);
                    crate::ci_checks::paint_mark(&painter, center, state, tab.ci.faded);
                    let hit = Rect::from_center_size(center, vec2(22.0, 22.0));
                    let mark = ui.interact(hit, Id::unique(("ci-mark", tab.id, commit.id.as_str())), Sense::click());
                    crate::pages::control(widgets, format!("ci-mark-{}", commit.id), &mark, ui.clip_rect());
                    if mark.on_hover_text(crate::ci_checks::mark_tip(state, tab.ci.host())).clicked() {
                        intents.push(Intent::CiChecks(commit.id.as_str().to_owned(), hit));
                    }
                }
                if show_author {
                    let author_clip = Rect::from_min_max(
                        pos2(rect.left() + message_end, rect.top()),
                        pos2(rect.right() - date_width - sha_width - 8.0, rect.bottom()),
                    );
                    let initials: String = display(&commit.author.name)
                        .split_whitespace()
                        .take(2)
                        .filter_map(|s| s.chars().next())
                        .collect();
                    let center = pos2(author_clip.left() + 18.0, rect.center().y);
                    painter.circle_filled(center, 10.0, AVATAR());
                    painter.text(
                        center,
                        Align2::CENTER_CENTER,
                        initials.to_uppercase(),
                        semibold(9.5),
                        TEXT(),
                    );
                    painter.with_clip_rect(author_clip).text(
                        pos2(author_clip.left() + 35.0, rect.center().y),
                        Align2::LEFT_CENTER,
                        display(&commit.author.name),
                        FontId::proportional(12.5),
                        MUTED(),
                    );
                }
                if date_width > 0.0 {
                    painter.text(
                        pos2(rect.right() - date_width - sha_width + 8.0, rect.center().y),
                        Align2::LEFT_CENTER,
                        date(commit.author.timestamp, commit.author.offset_minutes),
                        FontId::proportional(12.0),
                        FAINT(),
                    );
                }
                if sha_width > 0.0 {
                    painter.text(
                        pos2(rect.right() - sha_width + 8.0, rect.center().y),
                        Align2::LEFT_CENTER,
                        &commit.id.as_str()[..7],
                        FontId::monospace(12.0),
                        FAINT(),
                    );
                }
                if response.clicked() {
                    // Keyboard navigation continues from the clicked row.
                    response.request_focus();
                    let modifiers = ui.input(|i| i.modifiers);
                    intents.push(if modifiers.shift {
                        Intent::SelectCommitRange(commit.id.clone())
                    } else if modifiers.command {
                        Intent::ToggleCommit(commit.id.clone())
                    } else {
                        Intent::SelectCommit(commit.id.clone())
                    });
                }
                response.on_hover_text(format!(
                    "{}\n{}\n{}{}",
                    subject(commit),
                    commit.id,
                    tr("Ctrl-click or Shift-click to select several commits"),
                    if commit.parents.len() > 1 && query.is_empty() {
                        format!("\n{}", tr("← collapses this merge, → expands it (also in its right-click menu)"))
                    } else {
                        String::new()
                    }
                ));
            }
        });
    change_viewport(ui, widgets, "history-scroll", output.inner_rect);
    ui.spacing_mut().item_spacing.y = previous_spacing;
}
/// A commit row's right-click menu: collapse or expand this merge, or every merge.
fn fold_menu(
    ui: &mut Ui,
    tab: &Tab,
    commit: &Commit,
    intents: &mut Vec<Intent>,
    widgets: &mut std::collections::HashMap<String, UiControl>,
) {
    let r = ui.add_enabled(tab.selected_commits.len() == 2, Button::new(tr("Compare selected commits")));
    pages::control(widgets, "commit-menu-compare", &r, ui.clip_rect());
    if r.clicked() {
        intents.push(Intent::Compare(crate::compare::CompareIntent::Selected));
        ui.close();
    }
    // A preview's menu has only what reads; changes come with Keep.
    let preview = tab.is_preview();
    if !preview {
        let r = ui.button(tr("Export as commit series"));
        pages::control(widgets, "commit-menu-series", &r, ui.clip_rect());
        if r.clicked() {
            intents.push(Intent::ExportSeries(Some(series_range(tab, commit))));
            ui.close();
        }
    }
    let r = ui.button(tr("Search files at this commit"));
    pages::control(widgets, "commit-menu-search-files", &r, ui.clip_rect());
    if r.clicked() {
        intents.push(Intent::Tree(crate::tree::TreeIntent::SearchAt(commit.id.clone())));
        ui.close();
    }
    ui.separator();
    // The selection when the row is part of it, otherwise the row.
    let selected = tab.selected_commits.contains(&commit.id) || tab.selected_commit.as_ref() == Some(&commit.id);
    let free = tab.snapshot.as_ref().and_then(|s| s.state.as_ref()).is_some_and(OperationState::is_idle) && !tab.loading();
    let actions: &[(CommitOperation, &str)] = if preview {
        &[]
    } else {
        &[
            (CommitOperation::CherryPick, "commit-menu-cherry-pick"),
            (CommitOperation::Revert, "commit-menu-revert"),
        ]
    };
    for &(operation, key) in actions {
        let targets = if selected { tab.commit_targets(operation) } else { vec![commit.id.clone()] };
        let label = match (operation, targets.len()) {
            (CommitOperation::CherryPick, n @ 2..) => trf!("Cherry-pick {n} commits", n),
            (CommitOperation::CherryPick, _) => tr("Cherry-pick commit").to_owned(),
            (_, n @ 2..) => trf!("Revert {n} commits", n),
            _ => tr("Revert commit").to_owned(),
        };
        let r = ui.add_enabled(free, Button::new(label));
        pages::control(widgets, key, &r, ui.clip_rect());
        if r.clicked() {
            intents.push(Intent::Dialog(DialogKind::CommitAction(CommitActionRequest { operation, targets, mainline: None, recovery: None })));
            ui.close();
        }
    }
    if !actions.is_empty() {
        ui.separator();
    }
    if commit.parents.len() > 1 {
        let collapsed = tab.folding.collapsed(commit);
        let r = ui.button(if collapsed {
            tr("Expand merge")
        } else {
            tr("Collapse merge")
        });
        widgets.insert("commit-menu-fold".into(), (&r).into());
        if r.clicked() {
            intents.push(Intent::FoldMerge(commit.id.clone(), !collapsed));
            ui.close();
        }
        ui.separator();
    }
    for (collapse, label, key) in [
        (true, tr("Collapse all merges"), "commit-menu-collapse-all"),
        (false, tr("Expand all merges"), "commit-menu-expand-all"),
    ] {
        let r = ui.button(label);
        widgets.insert(key.into(), (&r).into());
        if r.clicked() {
            intents.push(Intent::FoldAllMerges(collapse));
            ui.close();
        }
    }
}
/// The selected commits' oldest and newest as short IDs, or `commit` alone when the
/// click was outside the selection. The list runs newest first.
fn series_range(tab: &Tab, commit: &Commit) -> (String, String) {
    let short = |id: &ObjectId| id.as_str()[..12].to_owned();
    if tab.selected_commits.len() < 2 || !tab.selected_commits.contains(&commit.id) {
        return (short(&commit.id), short(&commit.id));
    }
    let mut shown = tab.commits().iter().filter(|c| tab.selected_commits.contains(&c.id));
    let newest = shown.next().map_or(&commit.id, |c| &c.id);
    let oldest = shown.next_back().map_or(newest, |c| &c.id);
    (short(oldest), short(newest))
}
#[allow(clippy::too_many_arguments)]
pub(crate) fn changes(
    ui: &mut Ui,
    tab: &mut Tab,
    writable: bool,
    spelling: &mut crate::spell::Spelling,
    commit_height: &mut Option<f32>,
    intents: &mut Vec<Intent>,
    widgets: &mut std::collections::HashMap<String, UiControl>,
    commands: &[rocket_git::CustomCommand],
) {
    let status = tab
        .snapshot
        .as_ref()
        .and_then(|s| s.status.as_ref())
        .cloned();
    let Some(status) = status else {
        Frame::new().inner_margin(14).show(ui, |ui| {
            ui.label(tr("Changes require an available working tree."));
        });
        return;
    };
    let maximum = (ui.available_width() * 0.5).clamp(260.0, 480.0);
    let files = Panel::left(("changes-files", tab.id))
        .default_size((ui.available_width() * 0.3).clamp(270.0, 360.0))
        .size_range(260.0..=maximum)
        .resizable(true)
        .frame(Frame::new().fill(VIEW_PANEL()))
        .show(ui, |ui| {
            pane_header(
                ui,
                tr("Changes"),
                &trn!(status.entries.len(), "{n} file", "{n} files"),
            );
            if ui.available_height() < 300.0 {
                // Short panes cannot fit the file groups and a pinned form.
                // Keep their natural ordering in one viewport instead.
                let output = ScrollArea::vertical()
                    .id_salt(("changes-compact", tab.id))
                    .scroll_bar_visibility(scroll_area::ScrollBarVisibility::AlwaysVisible)
                    .auto_shrink([false, false])
                    .min_scrolled_height(0.0)
                    .show(ui, |ui| {
                        Frame::new()
                            .inner_margin(Margin::symmetric(6, 2))
                            .show(ui, |ui| {
                                change_groups(ui, tab, &status, writable, intents, widgets, commands);
                            });
                        Frame::new().inner_margin(12).show(ui, |ui| {
                            commit_editor(
                                ui, tab, &status, writable, None, spelling, intents, widgets,
                            );
                        });
                    });
                change_viewport(ui, widgets, "changes-scroll", output.inner_rect);
            } else {
                let area = commit_area(ui, tab, ui.available_height(), *commit_height);
                let editor = Panel::bottom(("commit-editor", tab.id))
                    .exact_size(area.height)
                    .resizable(false)
                    .frame(Frame::new().fill(VIEW_PANEL()).inner_margin(Margin { left: 12, right: 12, top: 0, bottom: 12 }))
                    .show(ui, |ui| {
                        let top = ui.min_rect().top();
                        if let Some(saved) = commit_grip(ui, tab.id, &area, widgets) {
                            *commit_height = saved;
                        }
                        let viewport = (area.height - area.fixed).max(2.0 * area.row);
                        commit_editor(
                            ui, tab, &status, writable, Some(viewport), spelling, intents, widgets,
                        );
                        // Everything but the description's lines, for the next frame's sizes.
                        let fixed = ui.min_rect().bottom() - top + 12.0 - viewport;
                        let key = Id::unique(("commit-fixed", tab.id));
                        if ui.ctx().data(|d| d.get_temp::<f32>(key)).is_none_or(|f| (f - fixed).abs() > 0.5) {
                            ui.ctx().data_mut(|d| d.insert_temp(key, fixed));
                            ui.ctx().request_repaint();
                        }
                    })
                    .response
                    .rect;
                ui.painter().hline(
                    editor.x_range(),
                    editor.top() + 0.5,
                    Stroke::new(1.0, HAIRLINE()),
                );
                let output = ScrollArea::vertical()
                    .id_salt(("change-groups", tab.id))
                    .auto_shrink([false, false])
                    .min_scrolled_height(0.0)
                    .show(ui, |ui| {
                        Frame::new()
                            .inner_margin(Margin::symmetric(6, 2))
                            .show(ui, |ui| {
                                change_groups(ui, tab, &status, writable, intents, widgets, commands)
                            });
                    });
                change_viewport(ui, widgets, "changes-scroll", output.inner_rect);
            }
        })
        .response
        .rect;
    ui.painter().vline(
        files.right() - 0.5,
        files.y_range(),
        Stroke::new(1.0, HAIRLINE()),
    );
    CentralPanel::default().frame(Frame::new().fill(VIEW_BG())).show(ui, |ui| {
        if let Some((path, staged)) = &tab.selected_change {
            let mut actions = |ui: &mut Ui, popup: bool| {
                let untracked=status.entries.iter().any(|e|e.path==*path&&e.kind==EntryKind::Untracked);
                let r=ui.add_enabled(writable&&(untracked||!*staged),quiet_unless(popup,Button::new(tr("Ignore…"))))
                    .on_hover_text(if untracked{tr("Append an exact ignore rule")}else{tr("Review stopping tracking while keeping the local file. Unstage selected changes first.")});
                widgets.insert("change-ignore".into(),(&r).into());if r.clicked(){intents.push(if untracked{Intent::Dialog(DialogKind::Ignore(vec![path.clone()]))}else{Intent::KeepLocal{snapshot:false,paths:vec![path.clone()]}});if popup{ui.close();}}
                let r = ui.add_enabled(writable && !*staged, quiet_unless(popup, Button::new(tr("Discard unstaged"))));
                widgets.insert("change-discard".into(), (&r).into());
                if r.clicked() {
                    intents.push(Intent::Dialog(DialogKind::FileEdit(Some((path.clone(), FileEditSource::Index)))));
                    if popup { ui.close(); }
                }
                let r = ui.add_enabled(writable, quiet_unless(popup, Button::new(tr("Restore from commit"))));
                widgets.insert("change-restore".into(), (&r).into());
                if r.clicked() {
                    intents.push(Intent::Dialog(DialogKind::FileEdit(Some((path.clone(), FileEditSource::Commit("HEAD".into()))))));
                    if popup { ui.close(); }
                }
                let r = ui.add(quiet_unless(popup, Button::new(tr("History & blame"))));
                widgets.insert("change-file-history".into(), (&r).into());
                if r.clicked() {
                    intents.push(Intent::File(crate::file_history::FileIntent::FromChange));
                    if popup { ui.close(); }
                }
            };
            let narrow = ui.available_width() < 480.0 || ui.available_height() < 300.0;
            let strip = Frame::new()
                .fill(VIEW_PANEL())
                .inner_margin(Margin::symmetric(8, 3))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        if narrow {
                            Some(ui.menu_button(tr("File actions"), |ui| actions(ui, true)).response)
                        } else {
                            ui.label(RichText::new(tr("File")).size(12.0).color(FAINT()));
                            ui.add_space(4.0);
                            actions(ui, false);
                            None
                        }
                    })
                    .inner
                });
            if let Some(menu) = &strip.inner {
                widgets.insert("change-file-actions".into(), menu.into());
            }
            let strip = strip.response.rect;
            ui.painter()
                .hline(strip.x_range(), strip.bottom() - 0.5, Stroke::new(1.0, HAIRLINE()));
        }
        // A rebase or import just paused: its first conflict is what to look at.
        if tab.show_conflict
            && tab.selected_change.is_none()
            && let Some(entry) = status.entries.iter().find(|e| e.kind == EntryKind::Unmerged)
        {
            tab.show_conflict = false;
            intents.push(Intent::SelectChange(entry.clone(), false));
        }
        if let Some((path, _)) = tab.selected_change.clone()
            && let Some(entry) = status.entries.iter().find(|e| e.path == path && e.kind == EntryKind::Unmerged) {
            // Binary files show as text conflicts until the editor reads them.
            let text = entry.submodule.is_none() && !matches!(entry.index, Change::Deleted) && !matches!(entry.worktree, Change::Deleted);
            Panel::top(("conflict-review",tab.id)).frame(Frame::new().fill(RED().gamma_multiply(0.08)).inner_margin(Margin::symmetric(14, 10))).show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(tr("Conflict")).font(semibold(13.0)).color(RED()));
                    let r = ui.add_enabled(writable, Button::new(RichText::new(tr("Open conflict editor")).color(ACCENT_INK())).fill(BLUE()));
                    widgets.insert("conflict-open".into(), (&r).into());
                    if r.clicked() { intents.push(Intent::Conflict(crate::conflicts::ConflictIntent::Open(path.clone()))); }
                    let r = crate::ai_conflict::resolve_button(ui, writable && text).on_disabled_hover_text(if text { tr("AI resolve is not available right now") } else { tr("AI resolve needs a text conflict") });
                    widgets.insert("conflict-open-ai".into(), (&r).into());
                    if r.clicked() { intents.push(Intent::Conflict(crate::conflicts::ConflictIntent::OpenAi(path.clone()))); }
                    ui.add_space(6.0);
                    for (side,label) in [(0,tr("Working file")),(1,tr("Base")),(2,tr("Ours")),(3,tr("Theirs"))] {
                        let r=ui.selectable_label(tab.conflict_side==side,label);
                        widgets.insert(format!("conflict-side-{side}"),(&r).into());
                        if r.clicked() { intents.push(Intent::ConflictSide(side)); }
                    }
                    ui.add_space(6.0);
                    let r=ui.add_enabled(writable,Button::new(tr("Mark resolved")));
                    widgets.insert("conflict-resolved".into(),(&r).into());
                    if r.clicked() { intents.push(Intent::Execute(tab.id,Action::MarkResolved(path))); }
                });
                let sides = crate::conflicts::sides(tab.snapshot.as_ref().and_then(|s| s.state.as_ref()));
                ui.label(RichText::new(sides).size(12.0).color(FAINT()));
            });
        }
        if let Some(intent) = patch_view(ui, tab, false, writable, widgets) { intents.push(intent); }
    });
}
/// A 40-point pane title row closed by a hairline.
fn pane_header(ui: &mut Ui, title: &str, detail: &str) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 40.0), Sense::hover());
    ui.painter().hline(
        rect.x_range(),
        rect.bottom() - 0.5,
        Stroke::new(1.0, HAIRLINE()),
    );
    let galley = ui
        .painter()
        .layout_no_wrap(title.into(), semibold(13.0), TEXT());
    let width = galley.size().x;
    ui.painter().galley(
        pos2(rect.left() + 14.0, rect.center().y - galley.size().y / 2.0),
        galley,
        TEXT(),
    );
    ui.painter().text(
        pos2(rect.left() + 22.0 + width, rect.center().y),
        Align2::LEFT_CENTER,
        detail,
        FontId::proportional(12.5),
        FAINT(),
    );
}
// Publish only the visible part of a Changes control. Automation must not
// click a scrolled-off form through the terminal or another panel beneath it.
fn change_control(
    widgets: &mut std::collections::HashMap<String, UiControl>,
    id: impl Into<String>,
    response: &Response,
) {
    if response.interact_rect.is_positive() {
        widgets.insert(
            id.into(),
            UiControl {
                rect: response.interact_rect,
                enabled: response.enabled(),
            },
        );
    }
}
fn change_viewport(
    ui: &Ui,
    widgets: &mut std::collections::HashMap<String, UiControl>,
    id: &str,
    rect: Rect,
) {
    let rect = rect.intersect(ui.clip_rect());
    if rect.is_positive() {
        widgets.insert(
            id.into(),
            UiControl {
                rect,
                enabled: ui.is_enabled(),
            },
        );
    }
}
/// Collapsible group title with an optional accent action (Stage all / Unstage all).
fn group_header(
    ui: &mut Ui,
    tab_id: u64,
    label: &str,
    count: usize,
    action: Option<(&str, bool)>,
) -> (bool, Option<Response>) {
    let id = Id::unique(("changes-group", tab_id, label));
    let mut expanded = ui.ctx().data_mut(|d| *d.get_persisted_mut_or(id, true));
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 32.0), Sense::hover());
    let action_width = action.map_or(0.0, |(text, _)| {
        ui.painter()
            .layout_no_wrap(text.into(), FontId::proportional(12.0), BLUE())
            .size()
            .x
            + 16.0
    });
    let toggle_rect = Rect::from_min_max(
        rect.min,
        pos2(rect.right() - action_width - 4.0, rect.bottom()),
    );
    let toggle = ui.interact(toggle_rect, id.with("toggle"), Sense::click());
    if toggle.clicked() {
        expanded = !expanded;
        ui.ctx().data_mut(|d| d.insert_persisted(id, expanded));
    }
    toggle.widget_info(|| {
        WidgetInfo::labeled(
            Role::Button,
            true,
            if expanded { trf!("Collapse {group}", group = label) } else { trf!("Expand {group}", group = label) },
        )
    });
    icon(
        ui.painter(),
        Rect::from_center_size(pos2(rect.left() + 12.0, rect.center().y), vec2(9.0, 9.0)),
        if expanded {
            Icon::Chevron
        } else {
            Icon::ChevronRight
        },
        if toggle.hovered() { TEXT() } else { FAINT() },
    );
    let galley = ui
        .painter()
        .layout_no_wrap(label.into(), semibold(12.0), MUTED());
    let width = galley.size().x;
    ui.painter().galley(
        pos2(rect.left() + 24.0, rect.center().y - galley.size().y / 2.0),
        galley,
        MUTED(),
    );
    ui.painter().text(
        pos2(rect.left() + 30.0 + width, rect.center().y),
        Align2::LEFT_CENTER,
        count.to_string(),
        FontId::proportional(12.0),
        FAINT(),
    );
    let response = action.map(|(text, enabled)| {
        let area = Rect::from_min_max(
            pos2(rect.right() - action_width, rect.top() + 4.0),
            pos2(rect.right(), rect.bottom() - 4.0),
        );
        let response = ui
            .scope_builder(UiBuilder::new().max_rect(area), |ui| {
                ui.add_enabled(enabled, Button::new("").frame(false).min_size(area.size()))
            })
            .inner;
        let active = response.enabled();
        if active && response.hovered() {
            ui.painter().rect_filled(area, 5.0, ACCENT_SOFT());
        }
        ui.painter().text(
            area.center(),
            Align2::CENTER_CENTER,
            text,
            FontId::proportional(12.0),
            if active { BLUE() } else { FAINT() },
        );
        response.widget_info(|| WidgetInfo::labeled(Role::Button, active, text));
        response
    });
    (expanded, response)
}
fn change_groups(
    ui: &mut Ui,
    tab: &mut Tab,
    status: &Status,
    writable: bool,
    intents: &mut Vec<Intent>,
    widgets: &mut std::collections::HashMap<String, UiControl>,
    commands: &[rocket_git::CustomCommand],
) {
    ui.spacing_mut().item_spacing.y = 1.0;
    tab.change_selection.reconcile(status);
    if let Some(notice) = &tab.change_selection.notice {
        ui.label(RichText::new(notice).size(12.0).color(RED()));
    }
    if tab.change_selection.paths.len() > 1 {
        ui.label(RichText::new(trf!("{n} selected · drag to the other group", n = tab.change_selection.paths.len())).size(12.0).color(BLUE()));
    }
    let conflicts: Vec<_> = status
        .entries
        .iter()
        .filter(|e| e.kind == EntryKind::Unmerged)
        .collect();
    let staged: Vec<_> = status
        .entries
        .iter()
        .filter(|e| {
            e.index != Change::Unchanged
                && !matches!(e.kind, EntryKind::Untracked | EntryKind::Unmerged)
        })
        .collect();
    let unstaged: Vec<_> = status
        .entries
        .iter()
        .filter(|e| {
            e.kind != EntryKind::Unmerged
                && (e.worktree != Change::Unchanged || e.kind == EntryKind::Untracked)
        })
        .collect();
    // A group can hold every file of a large working tree, so each one builds only the
    // rows in or next to the visible area.
    if !conflicts.is_empty() {
        let (expanded, _) = group_header(ui, tab.id, tr("Conflicts"), conflicts.len(), None);
        if expanded {
            crate::long_list::rows(ui, Id::unique(("change-rows", tab.id, "conflicts")), conflicts.len(), CHANGE_ROW, |ui, index| {
                change_row(ui, tab, ChangeRow { entry: conflicts[index], staged: false }, writable, intents, widgets, commands)
            });
        }
        ui.add_space(6.0);
    }
    let staged_group = ui.scope(|ui| {
        let (expanded, action) = group_header(
            ui,
            tab.id,
            tr("Staged"),
            staged.len(),
            Some((tr("Unstage all"), writable && !staged.is_empty())),
        );
        if let Some(r) = action {
            change_control(widgets, "unstage-all", &r);
            if r.clicked() {
                intents.push(Intent::Execute(tab.id, Action::UnstageAll));
            }
        }
        if expanded {
            if staged.is_empty() {
                empty_hint(ui, tr("No files staged"));
            }
            crate::long_list::rows(ui, Id::unique(("change-rows", tab.id, "staged")), staged.len(), CHANGE_ROW, |ui, index| {
                change_row(ui, tab, ChangeRow { entry: staged[index], staged: true }, writable, intents, widgets, commands)
            });
        }
    }).response.rect;
    if let Some(intent) = crate::change_drag::target(ui, staged_group, tab, status, true, writable, widgets) {
        intents.push(intent);
    }
    ui.add_space(6.0);
    let stage_enabled = writable
        && !unstaged.is_empty()
        && !status.entries.iter().any(|e| e.kind == EntryKind::Unmerged);
    let unstaged_group = ui.scope(|ui| {
        let (expanded, action) = group_header(
            ui,
            tab.id,
            tr("Unstaged"),
            unstaged.len(),
            Some((tr("Stage all"), stage_enabled)),
        );
        if let Some(r) = action {
            change_control(widgets, "stage-all", &r);
            if r.clicked() {
                intents.push(Intent::Execute(tab.id, Action::StageAll));
            }
        }
        if expanded {
            if unstaged.is_empty() {
                empty_hint(ui, tr("Working tree clean"));
            }
            crate::long_list::rows(ui, Id::unique(("change-rows", tab.id, "unstaged")), unstaged.len(), CHANGE_ROW, |ui, index| {
                change_row(ui, tab, ChangeRow { entry: unstaged[index], staged: false }, writable, intents, widgets, commands)
            });
        }
    }).response.rect;
    if let Some(intent) = crate::change_drag::target(ui, unstaged_group, tab, status, false, writable, widgets) {
        intents.push(intent);
    }
    crate::change_drag::scroll_edges(ui);
}
fn empty_hint(ui: &mut Ui, text: &str) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::hover());
    ui.painter().text(
        pos2(rect.left() + 24.0, rect.center().y),
        Align2::LEFT_CENTER,
        text,
        FontId::proportional(12.5),
        FAINT(),
    );
}
fn status_letter(entry: &StatusEntry, staged: bool) -> char {
    match entry.kind {
        EntryKind::Untracked => 'U',
        EntryKind::Unmerged => 'C',
        _ => match if staged { entry.index } else { entry.worktree } {
            Change::Added => 'A',
            Change::Deleted => 'D',
            Change::Renamed | Change::Copied => 'R',
            Change::Unmerged => 'C',
            _ => 'M',
        },
    }
}
/// Split a commit draft into its summary line and description.
/// The blank separator line belongs to neither field.
fn split_draft(draft: &str) -> (String, String) {
    match draft.split_once('\n') {
        None => (draft.to_owned(), String::new()),
        Some((summary, rest)) => (
            summary.trim_end_matches('\r').to_owned(),
            rest.strip_prefix("\r\n")
                .or_else(|| rest.strip_prefix('\n'))
                .unwrap_or(rest)
                .to_owned(),
        ),
    }
}
fn join_draft(summary: &str, description: &str) -> String {
    if description.is_empty() {
        summary.to_owned()
    } else {
        format!("{summary}\n\n{description}")
    }
}
/// The commit message area's height this frame and the limits of its handle.
struct CommitArea {
    height: f32,
    /// Everything in the area but the description's lines, measured last frame.
    fixed: f32,
    /// One line of the description.
    row: f32,
    min: f32,
    max: f32,
}

/// The handle strip above the summary.
const COMMIT_GRIP: f32 = 12.0;

/// Until dragged, the area opens at 7 lines of description (4 in a short pane) and
/// follows the text up to 12, then scrolls. A dragged height stays until reset.
fn commit_area(ui: &Ui, tab: &Tab, pane: f32, saved: Option<f32>) -> CommitArea {
    let row = ui.text_style_height(&TextStyle::Body);
    // First frame: the handle, margins, a 38-point summary, the controls below the
    // description and its frame. Later frames use what was measured.
    let fixed = ui.ctx().data(|d| d.get_temp::<f32>(Id::unique(("commit-fixed", tab.id)))).unwrap_or(184.0);
    let (_, description) = split_draft(&tab.draft);
    // The description wraps inside the area's margins, its frame and a scroll bar.
    let wrap = (ui.available_width() - 46.0).max(40.0);
    let rows = ui.painter().layout(description, TextStyle::Body.resolve(ui.style()), Color32::PLACEHOLDER, wrap).rows.len() as f32;
    let open = if pane < 600.0 { 4.0 } else { 7.0 };
    let min = fixed + 2.0 * row;
    // The file list keeps room for its header and a few rows.
    let max = (pane - 150.0).max(min);
    let dragging = ui.ctx().data(|d| d.get_temp::<f32>(Id::unique(("commit-drag", tab.id))));
    let height = match dragging.or(saved) {
        Some(height) => height.clamp(min, max),
        // Whole lines, so that when the pane limits the area its last line isn't cut.
        None => fixed + ((max - fixed) / row).floor().min(rows.clamp(open, 12.0)).max(2.0) * row,
    };
    CommitArea { height, fixed, row, min, max }
}

/// The handle above the commit message. Dragging resizes the area; the height is
/// saved when the drag ends, and a double-click goes back to following the text.
/// Returns the height to save when that changes.
fn commit_grip(
    ui: &mut Ui,
    tab: u64,
    area: &CommitArea,
    widgets: &mut std::collections::HashMap<String, UiControl>,
) -> Option<Option<f32>> {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), COMMIT_GRIP), Sense::click_and_drag());
    let response = response
        .on_hover_cursor(CursorIcon::ResizeVertical)
        .on_hover_text(tr("Drag to resize the commit message. Double-click for the default height."));
    let strong = response.hovered() || response.dragged();
    ui.painter().rect_filled(
        Rect::from_center_size(rect.center(), vec2(36.0, 4.0)),
        2.0,
        if strong { MUTED() } else { BORDER() },
    );
    change_control(widgets, "commit-resize", &response);
    let key = Id::unique(("commit-drag", tab));
    if response.double_clicked() {
        ui.ctx().data_mut(|d| d.remove::<f32>(key));
        return Some(None);
    }
    if response.dragged() {
        let height = (area.height - response.drag_delta().y).clamp(area.min, area.max);
        ui.ctx().data_mut(|d| d.insert_temp(key, height));
    }
    if response.drag_stopped() {
        let height = ui.ctx().data(|d| d.get_temp::<f32>(key)).unwrap_or(area.height);
        ui.ctx().data_mut(|d| d.remove::<f32>(key));
        let height = height.clamp(*crate::persistence::COMMIT_HEIGHTS.start(), *crate::persistence::COMMIT_HEIGHTS.end());
        return Some(Some(height));
    }
    None
}

#[allow(clippy::too_many_arguments)]
fn commit_editor(
    ui: &mut Ui,
    tab: &mut Tab,
    status: &Status,
    writable: bool,
    // The description's height in a pinned area; None in a short pane, where the
    // form scrolls with the file list.
    pinned: Option<f32>,
    spelling: &mut crate::spell::Spelling,
    intents: &mut Vec<Intent>,
    widgets: &mut std::collections::HashMap<String, UiControl>,
) {
    ui.spacing_mut().item_spacing.y = 8.0;
    let width = ui.available_width();
    let conflicts = status.entries.iter().any(|e| e.kind == EntryKind::Unmerged);
    let state = tab.snapshot.as_ref().and_then(|s| s.state.as_ref());
    // Recreating a merge during rebase also sets MERGE_HEAD. Recovery belongs
    // to the rebase banner; the ordinary commit form must not offer merge commit.
    let merge = state.is_some_and(|s| !s.merge_heads.is_empty() && !s.rebase && !s.mail_import);
    let idle = state.is_some_and(|s| s.is_idle());
    // A running Claude Code draft will replace the message: keep it read-only.
    let drafting = tab.composer.draft.running();
    let staged = status
        .entries
        .iter()
        .filter(|e| {
            e.index != Change::Unchanged
                && !matches!(e.kind, EntryKind::Untracked | EntryKind::Unmerged)
        })
        .count();
    let (mut summary, mut description) = split_draft(&tab.draft);
    // Summary: one line, counted against the conventional 72 characters.
    let field = |ui: &mut Ui, add: &mut dyn FnMut(&mut Ui) -> Response| {
        Frame::new()
            .fill(BG())
            .stroke(Stroke::new(1.0, BORDER()))
            .corner_radius(7)
            // The right margin holds the character counter inside the box.
            .inner_margin(Margin {
                left: 10,
                right: 54,
                top: 0,
                bottom: 0,
            })
            .show(ui, |ui| add(ui))
            .inner
    };
    let length = summary.chars().count();
    let summary_edit = field(ui, &mut |ui| {
        let color = ui
            .visuals()
            .override_text_color
            .unwrap_or_else(|| ui.visuals().widgets.inactive.text_color());
        crate::spell::show(
            ui,
            spelling,
            &mut summary,
            (semibold(14.0), color),
            widgets,
            |text| {
                TextEdit::singleline(text)
                    .id(Id::unique(("commit-message", tab.id)))
                    .interactive(!drafting)
                    .frame(Frame::NONE)
                    .font(semibold(14.0))
                    .hint_text(tr("Summary (required)"))
                    .align(Align2::LEFT_CENTER)
                    .min_size(vec2(width - 64.0, 36.0))
                    .desired_width(width - 64.0)
            },
        )
    });
    ui.painter().text(
        pos2(
            summary_edit.rect.right() + 44.0,
            summary_edit.rect.center().y,
        ),
        Align2::RIGHT_CENTER,
        format!("{length}/72"),
        FontId::proportional(11.0),
        if length > 72 { AMBER() } else { FAINT() },
    );
    change_control(widgets, "commit-message", &summary_edit);
    let compact = pinned.is_none();
    let description_height = pinned.unwrap_or(52.0);
    // Enough rows to fill the box, so a click anywhere in it reaches the text.
    let rows = if compact { 2 } else { ((description_height / ui.text_style_height(&TextStyle::Body)).floor() as usize).max(2) };
    // TextEdit's desired rows are a minimum, not a height limit. Bound the
    // viewport so a long draft cannot push the action outside its panel.
    let mut description_changed = false;
    Frame::new()
        .fill(BG())
        .stroke(Stroke::new(1.0, BORDER()))
        .corner_radius(7)
        .inner_margin(Margin::symmetric(10, 6))
        .show(ui, |ui| {
            let output = ScrollArea::vertical()
                .id_salt(("commit-message-scroll", tab.id))
                .auto_shrink([false, false])
                // Reserve this height even below the outer scroll viewport;
                // otherwise a nested ScrollArea collapses to an empty field.
                .min_scrolled_height(description_height)
                .max_height(description_height)
                .show(ui, |ui| {
                    // A commit template's comment lines guide an empty description,
                    // as they would in Git's editor; they are never committed.
                    let guidance = tab
                        .composer
                        .template
                        .as_ref()
                        .map(|t| t.guidance.as_str())
                        .filter(|g| !g.is_empty());
                    let body = TextStyle::Body.resolve(ui.style());
                    let edit = crate::spell::show(
                        ui,
                        spelling,
                        &mut description,
                        (body, MUTED()),
                        widgets,
                        |text| {
                            TextEdit::multiline(text)
                                .id(Id::unique(("commit-description", tab.id)))
                                .interactive(!drafting)
                                .desired_width(f32::INFINITY)
                                .desired_rows(rows)
                                .frame(Frame::NONE)
                                .text_color(MUTED())
                                .hint_text(guidance.unwrap_or(tr("Description (optional)")))
                        },
                    );
                    change_control(widgets, "commit-description", &edit);
                    description_changed = edit.changed();
                });
            change_viewport(ui, widgets, "commit-message-scroll", output.inner_rect);
        });
    if summary_edit.changed() || description_changed {
        tab.composer.edit_message(&mut tab.draft, join_draft(&summary, &description));
    }
    if let Some(error) = &tab.composer.message_error {
        ui.colored_label(AMBER(), error);
    }
    let branch = tab.branch_label();
    let target = trf!("to {branch}", branch);
    let target_width = ui.painter().layout_no_wrap(
        target.clone(), FontId::proportional(12.0), FAINT(),
    ).size().x;
    let mut target_below = false;
    ui.horizontal(|ui| {
        let amend = ui
            .add_enabled(
                writable && idle && !merge && !drafting,
                quiet(Button::new(RichText::new(tr("Amend last commit")).size(12.5))),
            )
            .on_hover_text(tr("Review an amended latest commit with the staged changes"));
        widgets.insert("composer-amend".into(), (&amend).into());
        if amend.clicked() {
            intents.push(Intent::Dialog(DialogKind::CommitAction(
                CommitActionRequest {
                    operation: CommitOperation::Amend,
                    targets: Vec::new(),
                    mainline: None,
                    recovery: None,
                },
            )));
        }
        ui.add_enabled_ui(!drafting, |ui| {
            crate::composer::message_menu(ui, &mut tab.draft, &mut tab.composer, widgets);
        });
        crate::commit_draft::composer_button(
            ui,
            &mut tab.composer.draft,
            writable && staged > 0,
            widgets,
        );
        // Translated controls can use the whole row even with a short branch.
        // Keep the destination visible instead of spending the last few pixels
        // on an ellipsis; long branch names can wrap on their own row.
        target_below = ui.available_width() < target_width + 1.0;
        if !target_below {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add(Label::new(RichText::new(&target).size(12.0).color(FAINT())));
            });
        }
    });
    if target_below {
        // A horizontal row constrains height to its contents. Applying the RTL
        // layout directly to the vertical editor would consume its remaining
        // height and push the commit buttons below the panel's viewport.
        ui.horizontal(|ui| {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add(Label::new(RichText::new(target).size(12.0).color(FAINT())).wrap());
            });
        });
    }
    let recover = state.and_then(|s| crate::chrome::Recover::of(s, tab.snapshot.as_ref().and_then(|t| t.sequence.as_ref())));
    // Push after commit: the reviewed Commit & Push takes the main button and Ctrl+Enter.
    let pushes = idle && !merge && crate::general::options(ui.ctx()).push_after_commit && tab.commit_pushes();
    let label = if state.is_some_and(|s| s.rebase) {
        tr("Continue rebase").to_owned()
    } else if state.is_some_and(|s| s.mail_import) {
        tr("Continue import").to_owned()
    } else if let Some(crate::chrome::Recover::Commit { operation, .. }) = recover {
        match operation {
            rocket_git::CommitOperation::CherryPick => tr("Continue cherry-pick").to_owned(),
            rocket_git::CommitOperation::Revert => tr("Continue revert").to_owned(),
            rocket_git::CommitOperation::Amend => tr("Continue amend").to_owned(),
        }
    } else if state.is_some_and(|s| s.merge_committed) {
        tr("Merge already committed").to_owned()
    } else if merge {
        tr("Continue merge").to_owned()
    } else if staged > 0 && pushes {
        trn!(staged, "Commit & Push {n} staged file", "Commit & Push {n} staged files")
    } else if staged > 0 {
        trn!(staged, "Commit {n} staged file", "Commit {n} staged files")
    } else if pushes {
        tr("Commit & Push").to_owned()
    } else {
        tr("Commit").to_owned()
    };
    let paused = tab.continue_paused();
    let enabled = writable && (tab.can_commit() || paused.is_some());
    let main_enabled = enabled && !(pushes && drafting);
    ui.horizontal(|ui| {
    let primary_width=if idle && !merge {(width-144.0).max(76.0)} else {width};
    let button = ui.add_enabled(
        main_enabled,
        Button::new("").frame(false).min_size(vec2(primary_width, 36.0)),
    );
    let rect = button.rect;
    ui.painter().rect_filled(
        rect,
        7.0,
        if !main_enabled {
            RAISED()
        } else if button.hovered() {
            ACCENT_HOVER()
        } else {
            BLUE()
        },
    );
    let ink = if main_enabled { ACCENT_INK() } else { FAINT() };
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        if primary_width<240.0 && idle && !merge {if pushes {tr("Commit & Push")} else {tr("Commit")}} else {&label},
        semibold(13.0),
        ink,
    );
    if primary_width>=280.0 { ui.painter().text(
        pos2(rect.right() - 12.0, rect.center().y),
        Align2::RIGHT_CENTER,
        "Ctrl+Enter",
        FontId::proportional(11.0),
        if main_enabled {
            ink.gamma_multiply(0.75)
        } else {
            FAINT().gamma_multiply(0.7)
        },
    ); }
    button.widget_info(|| WidgetInfo::labeled(Role::Button, main_enabled, &label));
    change_control(widgets, if pushes { "commit-push" } else { "commit" }, &button);
    if button.clicked() {
        intents.push(if let Some(paused) = paused {
            paused
        } else if pushes {
            Intent::CommitPush
        } else {
            Intent::Execute(
                tab.id,
                if merge {
                    Action::ContinueMerge(tab.draft.clone())
                } else {
                    Action::Commit(tab.draft.clone())
                },
            )
        });
    }
    if idle && !merge {
        let other=ui.add_enabled(enabled && !drafting,Button::new(if pushes {tr("Commit")} else {tr("Commit & Push")}).min_size(vec2(136.0,36.0)));
        change_control(widgets,if pushes {"commit"} else {"commit-push"},&other);
        if other.clicked(){intents.push(if pushes {Intent::Execute(tab.id,Action::Commit(tab.draft.clone()))} else {Intent::CommitPush});}
    }
    });
    if crate::commit_draft::composer_status(ui, &mut tab.composer.draft, &mut tab.draft, widgets) {
        return;
    }
    ui.label(
        RichText::new(if recover.is_some() {
            if enabled {
                tr("Shows what happens next before anything changes.")
            } else if conflicts {
                tr("Fix and stage the conflict first.")
            } else {
                tr("The banner above says what to do next.")
            }
        } else if state.is_some_and(|s| s.merge_committed) {
            tr("Use Clear merge state above. Your commit, index and files stay in place.")
        } else if conflicts {
            tr("Resolve conflicts before committing.")
        } else if staged == 0 && !merge {
            tr("Stage files to include them in the commit.")
        } else {
            tr("Uses this repository's Git identity, hooks and signing.")
        })
        .size(11.5)
        .color(FAINT()),
    );
}
struct ChangeRow<'a> {
    entry: &'a StatusEntry,
    staged: bool,
}

/// The height of one file row in the Changes list.
const CHANGE_ROW: f32 = 30.0;

/// Shows one file row. Returns whether the row must stay built once it is out of view:
/// it holds keyboard focus, an open menu or a drag.
fn change_row(
    ui: &mut Ui,
    tab: &mut Tab,
    row: ChangeRow<'_>,
    writable: bool,
    intents: &mut Vec<Intent>,
    widgets: &mut std::collections::HashMap<String, crate::app::UiControl>,
    commands: &[rocket_git::CustomCommand],
) -> bool {
    let ChangeRow { entry, staged } = row;
    let response = ui.add(
        Button::new("")
            .frame(false)
            .min_size(vec2(ui.available_width(), CHANGE_ROW)),
    ).interact(Sense::drag());
    let response = if writable && crate::change_drag::belongs(entry, staged) {
        response.on_hover_cursor(CursorIcon::Grab)
    } else { response };
    crate::change_drag::source(ui, &response, tab, entry, staged, writable);
    // The list also builds the first and last row and the neighbours of a focused one
    // while they are out of view, for keyboard focus to move to. Those idle rows need
    // no path formatting, text layout, actions or paint commands.
    if !ui.is_rect_visible(response.rect)
        && !response.has_focus()
        && !response.context_menu_opened()
        && !response.dragged()
    {
        return false;
    }
    let selected = if entry.kind != EntryKind::Unmerged && tab.change_selection.staged.is_some() {
        tab.change_selection.contains(&entry.path, staged)
    } else {
        tab.selected_change.as_ref().is_some_and(|(p, s)| p == &entry.path && *s == staged)
    };
    let label = entry.path.display_lossy();
    let (dir, name) = match label.rsplit_once('/') {
        Some((dir, name)) => (dir.to_owned(), name.to_owned()),
        None => (String::new(), label.to_string()),
    };
    let detail = match &entry.original_path {
        Some(original) => trf!("from {path}", path = original.display_lossy()),
        None => dir,
    };
    let rect = response.rect;
    let path_id: String = entry
        .path
        .as_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    change_control(
        widgets,
        format!(
            "change-{}-{path_id}",
            if staged { "staged" } else { "unstaged" }
        ),
        &response,
    );
    let conflict = entry.kind == EntryKind::Unmerged;
    // Row actions appear on hover or selection, painted over the row.
    let hot = response.hovered() || selected || response.has_focus();
    let mut actions: Vec<(Icon, &str, Action)> = Vec::new();
    let mut paths = vec![entry.path.clone()];
    if let Some(original) = &entry.original_path {
        paths.push(original.clone());
    }
    if conflict {
        actions.push((
            Icon::Check,
            tr("Mark resolved after editing the working file"),
            Action::MarkResolved(entry.path.clone()),
        ));
    } else if staged {
        actions.push((Icon::Chevron, tr("Unstage file"), Action::Unstage(paths)));
    } else {
        actions.push((Icon::Plus, tr("Stage file"), Action::Stage(paths)));
    }
    let actions_width = actions.len() as f32 * 26.0 + 4.0;
    style::row_background(
        ui.painter(),
        rect,
        selected,
        response.hovered() || response.has_focus(),
    );
    status_badge(
        ui.painter(),
        pos2(rect.left() + 17.0, rect.center().y),
        status_letter(entry, staged),
    );
    let right = rect.right() - if hot { actions_width } else { 6.0 };
    let name_galley = ui
        .painter()
        .layout_no_wrap(name.clone(), FontId::proportional(13.0), TEXT());
    let name_rect = Rect::from_min_max(
        pos2(rect.left() + 34.0, rect.top()),
        pos2(right, rect.bottom()),
    );
    let name_width = name_galley.size().x.min(name_rect.width());
    ui.painter().with_clip_rect(name_rect).galley(
        pos2(
            name_rect.left(),
            rect.center().y - name_galley.size().y / 2.0,
        ),
        name_galley,
        TEXT(),
    );
    if entry.kind == EntryKind::Tracked && !staged && entry.worktree == Change::Deleted
        || staged && entry.index == Change::Deleted
    {
        ui.painter().hline(
            name_rect.left()..=name_rect.left() + name_width,
            rect.center().y,
            Stroke::new(1.0, RED().gamma_multiply(0.7)),
        );
    }
    let detail_left = name_rect.left() + name_width + 8.0;
    if !detail.is_empty() && detail_left < right - 20.0 {
        clipped_text(
            ui,
            Rect::from_min_max(pos2(detail_left, rect.top()), pos2(right, rect.bottom())),
            &detail,
            12.0,
            FAINT(),
        );
    }
    let mut action_clicked = false;
    if hot {
        for (i, (image, hint, action)) in actions.into_iter().enumerate() {
            let button = Rect::from_center_size(
                pos2(rect.right() - 16.0 - i as f32 * 26.0, rect.center().y),
                vec2(24.0, 24.0),
            );
            let r = ui
                .interact(
                    button,
                    Id::unique(("change-action", tab.id, staged, &path_id, i)),
                    Sense::click(),
                )
                .on_hover_text(hint);
            let enabled = writable;
            if enabled && r.hovered() {
                ui.painter().rect_filled(button, 5.0, RAISED());
            }
            let color = if !enabled {
                FAINT().gamma_multiply(0.6)
            } else if r.hovered() {
                TEXT()
            } else {
                MUTED()
            };
            if matches!(image, Icon::Chevron) {
                // Unstage: a minus sign.
                ui.painter().hline(
                    button.center().x - 5.0..=button.center().x + 5.0,
                    button.center().y,
                    Stroke::new(1.4, color),
                );
            } else {
                icon(ui.painter(), button.shrink(6.0), image, color);
            }
            r.widget_info(|| WidgetInfo::labeled(Role::Button, enabled, hint));
            if r.clicked() && enabled {
                action_clicked = true;
                intents.push(Intent::Execute(tab.id, action));
            }
        }
    }
    if response.clicked() && !action_clicked {
        if conflict {
            tab.change_selection = crate::change_drag::Selection::default();
        } else if let Some(status) = tab.snapshot.as_ref().and_then(|s| s.status.as_ref()) {
            tab.change_selection.click(&entry.path, staged, status, ui.input(|i| i.modifiers));
        }
        intents.push(Intent::SelectChange(entry.clone(), staged));
    }
    response.context_menu(|ui| {
        let mut context = crate::custom_commands::context(tab);
        context.file = std::str::from_utf8(entry.path.as_bytes()).ok().map(|p| tab.path.join(p));
        crate::custom_commands::context_menu(ui, commands, rocket_git::CommandScope::File, &context, intents, widgets);
        if ui.button(tr("Show changes")).clicked() {
            intents.push(Intent::SelectChange(entry.clone(), staged));
            ui.close();
        }
        ui.separator();
        if !conflict {
            let mut paths = vec![entry.path.clone()];
            if let Some(original) = &entry.original_path {
                paths.push(original.clone());
            }
            if ui
                .add_enabled(
                    writable,
                    Button::new(if staged { tr("Unstage file") } else { tr("Stage file") }),
                )
                .clicked()
            {
                intents.push(Intent::Execute(
                    tab.id,
                    if staged {
                        Action::Unstage(paths)
                    } else {
                        Action::Stage(paths)
                    },
                ));
                ui.close();
            }
        }
        if ui
            .add_enabled(
                writable && !staged && !conflict,
                Button::new(tr("Discard unstaged")),
            )
            .clicked()
        {
            intents.push(Intent::Dialog(DialogKind::FileEdit(Some((
                entry.path.clone(),
                FileEditSource::Index,
            )))));
            ui.close();
        }
        if ui
            .add_enabled(writable, Button::new(tr("Restore from commit")))
            .clicked()
        {
            intents.push(Intent::Dialog(DialogKind::FileEdit(Some((
                entry.path.clone(),
                FileEditSource::Commit("HEAD".into()),
            )))));
            ui.close();
        }
        ui.separator();
        if ui.button(tr("History & blame")).clicked() {
            intents.push(Intent::SelectChange(entry.clone(), staged));
            intents.push(Intent::File(crate::file_history::FileIntent::FromChange));
            ui.close();
        }
    });
    let stay = response.has_focus() || response.context_menu_opened() || response.dragged();
    response.on_hover_text(format!(
        "{}\n{}: {:?} · {}: {:?}",
        label, tr("Index"), entry.index, tr("Working tree"), entry.worktree
    ));
    stay
}
/// Branches page: local and remote-tracking branches beside the selection's details.
/// The selection is view state only and falls back to HEAD when its branch is gone.
pub(crate) fn branches(
    ui: &mut Ui,
    tab: &Tab,
    writable: bool,
    intents: &mut Vec<Intent>,
    widgets: &mut std::collections::HashMap<String, UiControl>,
    commands: &[rocket_git::CustomCommand],
) {
    let Some(snapshot) = &tab.snapshot else {
        return;
    };
    // The current branch leads, as in the sidebar.
    let mut local: Vec<_> = snapshot
        .branches
        .iter()
        .enumerate()
        .filter(|(_, b)| b.kind == BranchKind::Local)
        .collect();
    local.sort_by_key(|(_, b)| !b.is_head);
    let tracking: Vec<_> = snapshot
        .branches
        .iter()
        .enumerate()
        .filter(|(_, b)| b.kind == BranchKind::Remote)
        .collect();
    let key = Id::unique(("branches-selected", tab.id));
    let stored: Option<Vec<u8>> = ui.data(|d| d.get_temp(key));
    let selected = stored
        .and_then(|r| snapshot.branches.iter().find(|b| b.reference == r))
        .or_else(|| snapshot.branches.iter().find(|b| b.is_head))
        .or_else(|| snapshot.branches.first());
    let ahead_behind = snapshot.status.as_ref().and_then(|s| s.ahead_behind);
    let short = ui.available_height() < 360.0;
    pages::page_title(
        ui,
        short,
        Icon::Branch,
        tr("Branches"),
        &trf!(
            "{local} local · {tracking} remote-tracking · on {head}",
            local = local.len(),
            tracking = tracking.len(),
            head = head_name(snapshot.head.as_ref())
        ),
        |ui| {
            if pages::primary(ui, writable, tr("New branch…")).clicked() {
                intents.push(Intent::Dialog(DialogKind::NewBranch));
            }
        },
    );
    let mut select = None;
    pages::split(ui, |ui, side| match side {
        Side::List => {
            ScrollArea::vertical()
                .id_salt(("branches-list", tab.id))
                .max_height(ui.available_height())
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    if snapshot.branches.is_empty() {
                        pages::empty(ui, tr("No branches yet. Create the first commit to start one."));
                    }
                    for (key, title, group) in [("local", tr("Local"), &local), ("tracking", tr("Remote-tracking"), &tracking)] {
                        if group.is_empty() {
                            continue;
                        }
                        ui.add_space(4.0);
                        pages::caption(ui, &format!("{title} · {}", group.len()));
                        // A repository can have thousands of branches: build only the
                        // rows in or next to the visible area.
                        crate::long_list::rows(ui, Id::unique(("branch-rows", tab.id, key)), group.len(), BRANCH_ROW, |ui, row| {
                            let (index, branch) = group[row];
                            let current = selected.is_some_and(|s| s.reference == branch.reference);
                            let r = branch_row(ui, branch, current, ahead_behind);
                            pages::control(
                                widgets,
                                format!("branch-row-{index}"),
                                &r,
                                ui.clip_rect(),
                            );
                            if r.clicked() {
                                select = Some(branch.reference.clone());
                            }
                            branch_menu(&r, tab, branch, writable, intents, commands, widgets);
                            r.has_focus() || r.context_menu_opened()
                        });
                        ui.add_space(10.0);
                    }
                });
        }
        Side::Details => {
            let Some(branch) = selected else {
                pages::empty(
                    ui,
                    tr("Select a branch to see its commit, upstream and actions."),
                );
                return;
            };
            branch_details(ui, tab.id, branch, ahead_behind, writable, intents, widgets);
        }
    });
    if let Some(reference) = select {
        ui.data_mut(|d| d.insert_temp(key, reference));
    }
}
fn local_name(branch: &Branch) -> Vec<u8> {
    branch
        .reference
        .strip_prefix(b"refs/heads/")
        .unwrap_or(&branch.reference)
        .to_vec()
}
/// The history order's name, which Git's side of Rocket keeps in English.
fn order_label(order: HistoryOrder) -> &'static str {
    match order {
        HistoryOrder::Topological => tr("Topological"),
        HistoryOrder::CommitDate => tr("Commit date"),
        HistoryOrder::AuthorDate => tr("Author date"),
    }
}
fn delete_branch(branch: &Branch) -> Intent {
    Intent::Dialog(DialogKind::Confirm(
        Action::DeleteBranch(local_name(branch)),
        trf!(
            "Delete local branch {label}? Git will refuse unmerged branches.",
            label = short_ref(&branch.reference)
        ),
    ))
}
fn show_history(branch: &Branch, intents: &mut Vec<Intent>) {
    if let Ok(reference) = String::from_utf8(branch.reference.clone()) {
        intents.push(Intent::History(Some(reference)));
        intents.push(Intent::View(View::Commits));
    }
}
/// The height of one row of the Branches page, as [`pages::row`] builds it.
const BRANCH_ROW: f32 = 46.0;

fn branch_row(
    ui: &mut Ui,
    branch: &Branch,
    selected: bool,
    ahead_behind: Option<(u64, u64)>,
) -> Response {
    let name = short_ref(&branch.reference);
    let local = branch.kind == BranchKind::Local;
    let mut detail = branch.target.as_str()[..8].to_owned();
    if let Some(target) = &branch.symbolic_target {
        detail = trf!("points to {target}", target = short_ref(target));
    } else if let Some(upstream) = &branch.upstream {
        detail.push_str(&format!(" · {}", trf!("tracks {upstream}", upstream = short_ref(upstream))));
    } else if local {
        detail.push_str(&format!(" · {}", tr("no upstream")));
    }
    let mut pills = Vec::new();
    if branch.is_head {
        pills.push((tr("Current").to_owned(), pages::INFO()));
        if let Some((ahead, behind)) = ahead_behind
            && ahead + behind > 0
        {
            pills.push((format!("↑{ahead} ↓{behind}"), pages::NEUTRAL()));
        }
    } else if branch.checked_out_at.is_some() {
        pills.push((tr("Other worktree").to_owned(), pages::NEUTRAL()));
    }
    pages::row(
        ui,
        pages::Row {
            image: if local { Icon::Branch } else { Icon::Remote },
            title: &name,
            detail: &detail,
            pills,
            indent: 0.0,
        },
        selected,
        true,
    )
    .on_hover_text(format!(
        "{}\n{}{}",
        display(&branch.reference),
        branch.target,
        if local {
            tr("\nRight-click for actions")
        } else {
            ""
        }
    ))
}
/// The same actions as the sidebar's branch menu.
fn branch_menu(r: &Response, owner: &Tab, branch: &Branch, writable: bool, intents: &mut Vec<Intent>, commands: &[rocket_git::CustomCommand], widgets: &mut std::collections::HashMap<String, UiControl>) {
    let tab = owner.id;
    let local = branch.kind == BranchKind::Local;
    r.context_menu(|ui| {
        let mut context = crate::custom_commands::context(owner);
        context.branch = String::from_utf8(branch.reference.clone()).ok();
        crate::custom_commands::context_menu(ui, commands, rocket_git::CommandScope::Branch, &context, intents, widgets);
        let target = crate::integrations::CommandTarget::branch(&owner.path, &branch.reference);
        crate::integrations::context_rows(ui, &target, crate::integrations::Divider::After, intents, widgets);
        if local
            && ui
                .add_enabled(writable && !branch.is_head, Button::new(tr("Switch to branch")))
                .clicked()
        {
            intents.push(Intent::Execute(
                tab,
                Action::SwitchBranch(local_name(branch)),
            ));
            ui.close();
        }
        if local && ui.add_enabled(writable && !branch.is_head && std::str::from_utf8(&branch.reference).is_ok(),Button::new(tr("Save, switch & restore…"))).clicked() {
            if let Ok(name)=String::from_utf8(local_name(branch)){intents.push(Intent::SavedSwitch(name));}
            ui.close();
        }
        if ui.button(tr("Show history")).clicked() {
            show_history(branch, intents);
            ui.close();
        }
        if let Ok(reference) = std::str::from_utf8(&branch.reference)
            && ui.button(tr("Compare with HEAD")).clicked() {
            intents.push(Intent::Compare(crate::compare::CompareIntent::Branch(reference.into())));
            ui.close();
        }
        if local {
            ui.separator();
            if ui
                .add_enabled(
                    writable && !branch.is_head,
                    Button::new(tr("Merge into current branch")),
                )
                .clicked()
            {
                if let Ok(reference) = String::from_utf8(branch.reference.clone()) {
                    intents.push(Intent::MergeBranch(reference));
                }
                ui.close();
            }
            if ui.add_enabled(writable, Button::new(tr("Rename"))).clicked() {
                intents.push(Intent::Dialog(DialogKind::Rename(local_name(branch))));
                ui.close();
            }
            ui.separator();
            if ui
                .add_enabled(
                    writable && !branch.is_head,
                    Button::new(RichText::new(tr("Delete branch")).color(RED())),
                )
                .clicked()
            {
                intents.push(delete_branch(branch));
                ui.close();
            }
        }
    });
}
fn branch_details(
    ui: &mut Ui,
    tab: u64,
    branch: &Branch,
    ahead_behind: Option<(u64, u64)>,
    writable: bool,
    intents: &mut Vec<Intent>,
    widgets: &mut std::collections::HashMap<String, UiControl>,
) {
    let local = branch.kind == BranchKind::Local;
    ui.add(Label::new(RichText::new(short_ref(&branch.reference)).font(semibold(16.0))).wrap());
    pages::note(
        ui,
        if !local {
            tr("Remote-tracking branch · moved by fetch, never edited here")
        } else if branch.is_head {
            tr("Current branch")
        } else {
            tr("Local branch")
        },
    );
    ui.add_space(4.0);
    pages::field(ui, tr("Reference"), &display(&branch.reference), true);
    pages::field(ui, tr("Commit"), branch.target.as_str(), true);
    if let Some(target) = &branch.symbolic_target {
        pages::field(ui, tr("Points to"), &short_ref(target), false);
    }
    if local {
        let upstream = branch
            .upstream
            .as_ref()
            .map_or_else(|| tr("None").into(), |u| short_ref(u));
        pages::field(ui, tr("Upstream"), &upstream, false);
    }
    if branch.is_head
        && let Some((ahead, behind)) = ahead_behind
    {
        pages::field(
            ui,
            tr("Upstream sync"),
            &trf!("{ahead} ahead · {behind} behind", ahead, behind),
            false,
        );
    }
    if let Some(path) = &branch.checked_out_at {
        pages::field(ui, tr("Checked out at"), &path.display().to_string(), false);
    }
    ui.add_space(10.0);
    ui.horizontal_wrapped(|ui| {
        if local {
            let r = ui.add_enabled(
                writable && !branch.is_head && std::str::from_utf8(&branch.reference).is_ok(),
                Button::new(tr("Merge into current")),
            );
            pages::control(widgets, "branch-merge", &r, ui.clip_rect());
            if r.clicked()
                && let Ok(reference) = String::from_utf8(branch.reference.clone())
            {
                intents.push(Intent::MergeBranch(reference));
            }
            let switch=pages::primary(ui, writable && !branch.is_head, tr("Switch to branch"));
            pages::control(widgets,"branch-switch",&switch,ui.clip_rect());
            let saved=ui.add_enabled(writable && !branch.is_head && std::str::from_utf8(&branch.reference).is_ok(),Button::new(tr("Save, switch & restore…")));
            pages::control(widgets,"branch-saved-switch",&saved,ui.clip_rect());
            if saved.clicked() && let Ok(name)=String::from_utf8(local_name(branch)){intents.push(Intent::SavedSwitch(name));}
            if switch.clicked() {
                intents.push(Intent::Execute(
                    tab,
                    Action::SwitchBranch(local_name(branch)),
                ));
            }
            if ui.add_enabled(writable, Button::new(tr("Rename"))).clicked() {
                intents.push(Intent::Dialog(DialogKind::Rename(local_name(branch))));
            }
        }
        let r = ui.add_enabled(std::str::from_utf8(&branch.reference).is_ok(), Button::new(tr("Compare with HEAD")));
        pages::control(widgets, "branch-compare", &r, ui.clip_rect());
        if r.clicked() && let Ok(reference) = std::str::from_utf8(&branch.reference) {
            intents.push(Intent::Compare(crate::compare::CompareIntent::Branch(reference.into())));
        }
        if ui
            .add_enabled(
                std::str::from_utf8(&branch.reference).is_ok(),
                Button::new(tr("Show history")),
            )
            .clicked()
        {
            show_history(branch, intents);
        }
        if local && pages::danger_ghost(ui, writable && !branch.is_head, tr("Delete…")).clicked() {
            intents.push(delete_branch(branch));
        }
    });
    if local {
        pages::note(
            ui,
            if branch.is_head {
                tr("Switch to another branch before deleting this one.")
            } else {
                tr("Delete asks first; Git refuses to delete a branch that is not merged.")
            },
        );
    }
}

/// Remotes page: configured remotes beside the selection's details and actions.
/// Configured URLs stay hidden until explicitly shown because they can hold credentials.
pub(crate) fn remotes(
    ui: &mut Ui,
    tab: &Tab,
    enabled: bool,
    intents: &mut Vec<Intent>,
    widgets: &mut std::collections::HashMap<String, UiControl>,
) {
    let Some(snapshot) = &tab.snapshot else {
        return;
    };
    let key = Id::unique(("remotes-selected", tab.id));
    let stored: Option<Vec<u8>> = ui.data(|d| d.get_temp(key));
    let selected = stored
        .and_then(|name| snapshot.remotes.iter().position(|r| r.name == name))
        .or((!snapshot.remotes.is_empty()).then_some(0));
    let tracking = |remote: &Remote| {
        let prefix = format!("{}/", display(&remote.name));
        snapshot
            .branches
            .iter()
            .filter(|b| {
                b.kind == BranchKind::Remote
                    && b.symbolic_target.is_none()
                    && short_ref(&b.reference).starts_with(&prefix)
            })
            .count()
    };
    let short = ui.available_height() < 360.0;
    pages::page_title(
        ui,
        short,
        Icon::Remote,
        tr("Remotes"),
        &trf!(
            "{n} configured · editing a remote does not contact its server",
            n = snapshot.remotes.len()
        ),
        |ui| {
            let add = pages::primary(ui, enabled, tr("Add remote…"));
            pages::control(widgets, "remote-add", &add, ui.clip_rect());
            if add.clicked() {
                intents.push(Intent::Dialog(DialogKind::AddRemote));
            }
        },
    );
    let mut select = None;
    pages::split(ui, |ui, side| match side {
        Side::List => {
            ScrollArea::vertical()
                .id_salt(("remotes-list", tab.id))
                .max_height(ui.available_height())
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    if snapshot.remotes.is_empty() {
                        pages::empty(
                            ui,
                            tr("No remotes configured. Add a remote to connect this repository."),
                        );
                    }
                    for (i, remote) in snapshot.remotes.iter().enumerate() {
                        let name = display(&remote.name);
                        let branches = tracking(remote);
                        let detail = format!(
                            "{} · {}",
                            trn!(branches, "{n} remote-tracking branch", "{n} remote-tracking branches"),
                            if remote.push_urls.is_empty() {
                                tr("push uses the fetch address")
                            } else {
                                tr("separate push address")
                            }
                        );
                        let r = pages::row(
                            ui,
                            pages::Row {
                                image: Icon::Remote,
                                title: &name,
                                detail: &detail,
                                pills: Vec::new(),
                                indent: 0.0,
                            },
                            selected == Some(i),
                            true,
                        );
                        pages::control(widgets, format!("remote-row-{i}"), &r, ui.clip_rect());
                        if r.clicked() {
                            select = Some(remote.name.clone());
                        }
                    }
                });
        }
        Side::Details => {
            let Some(i) = selected else {
                pages::empty(
                    ui,
                    tr("Add a remote to fetch from or push to another repository."),
                );
                return;
            };
            let remote = &snapshot.remotes[i];
            ui.add(Label::new(RichText::new(display(&remote.name)).font(semibold(16.0))).wrap());
            pages::note(
                ui,
                if remote.push_urls.is_empty() {
                    tr("Push uses the fetch address")
                } else {
                    tr("Separate fetch and push addresses")
                },
            );
            ui.add_space(4.0);
            let urls = |n: usize| if n == 0 { tr("None").to_owned() } else { trn!(n, "{n} URL", "{n} URLs") };
            pages::field(ui, tr("Fetch"), &urls(remote.urls.len()), false);
            pages::field(
                ui,
                tr("Push"),
                &if remote.push_urls.is_empty() {
                    tr("Fetch URL").to_owned()
                } else {
                    urls(remote.push_urls.len())
                },
                false,
            );
            pages::field(
                ui,
                tr("Tracking"),
                &match tracking(remote) { 0 => tr("None").to_owned(), n => trn!(n, "{n} branch", "{n} branches") },
                false,
            );
            for refspec in &remote.fetch_refspecs {
                pages::field(ui, tr("Refspec"), &display(refspec), true);
            }
            ui.add_space(10.0);
            ui.horizontal_wrapped(|ui| {
                let fetch = pages::primary(ui, enabled, tr("Fetch"));
                pages::control(widgets, format!("remote-fetch-{i}"), &fetch, ui.clip_rect());
                if fetch.clicked() {
                    intents.push(Intent::Execute(tab.id, Action::Fetch(remote.name.clone())));
                }
                for (kind, label, key, values) in [
                    (RemoteUrlKind::Fetch, tr("Edit fetch URL"), "remote-edit-fetch", &remote.urls),
                    (RemoteUrlKind::Push, tr("Edit push URL"), "remote-edit-push", &remote.push_urls),
                ] {
                    let editable = values.len() <= 1
                        && values.iter().all(|v| std::str::from_utf8(v).is_ok());
                    let edit = ui
                        .add_enabled(enabled && editable, Button::new(label))
                        .on_disabled_hover_text(
                            tr("Multiple or non-UTF-8 URLs must be edited with Git so their original values are preserved."),
                        );
                    pages::control(widgets, format!("{key}-{i}"), &edit, ui.clip_rect());
                    if edit.clicked() {
                        intents.push(Intent::Dialog(DialogKind::EditRemote(remote.clone(), kind)));
                    }
                }
                let remove = pages::danger_ghost(ui, enabled, tr("Remove…"));
                pages::control(widgets, format!("remote-remove-{i}"), &remove, ui.clip_rect());
                if remove.clicked() {
                    intents.push(Intent::Dialog(DialogKind::Confirm(
                        Action::RemoveRemote(remote.clone()),
                        trf!(
                            "Remove remote {remote} and its remote-tracking branches? Nothing is deleted from the server.",
                            remote = display(&remote.name)
                        ),
                    )));
                }
            });
            ui.add_space(6.0);
            // URLs can contain credentials. Explicit local disclosure only, per remote,
            // so changing the selection never reveals another remote's addresses.
            CollapsingHeader::new(tr("Show configured URLs (may contain credentials)"))
                .id_salt(("remote-urls", &remote.name))
                .show(ui, |ui| {
                    for url in &remote.urls {
                        pages::field(ui, tr("Fetch"), &display(url), true);
                    }
                    for url in &remote.push_urls {
                        pages::field(ui, tr("Push"), &display(url), true);
                    }
                    if remote.push_urls.is_empty() {
                        pages::note(ui, tr("Push falls back to the fetch URL."));
                    }
                });
        }
    });
    if let Some(name) = select {
        ui.data_mut(|d| d.insert_temp(key, name));
    }
}

#[cfg(test)]
mod tests {
    use super::{Branch, BranchKind, Chip, Remote, branch_chips, display_path, join_draft, split_draft};
    use rocket_git::ObjectId;
    use std::path::Path;

    fn branch(reference: &str, upstream: Option<&str>, is_head: bool) -> Branch {
        Branch {
            reference: reference.as_bytes().to_vec(),
            target: ObjectId::parse(&"a".repeat(40)).unwrap(),
            kind: if reference.starts_with("refs/remotes/") { BranchKind::Remote } else { BranchKind::Local },
            is_head,
            upstream: upstream.map(|u| u.as_bytes().to_vec()),
            symbolic_target: None,
            checked_out_at: None,
            committed: None,
            ahead_behind: None,
        }
    }
    fn remote(name: &str) -> Remote {
        Remote { name: name.as_bytes().to_vec(), urls: Vec::new(), push_urls: Vec::new(), fetch_refspecs: Vec::new() }
    }
    fn labels(chips: &[Chip]) -> Vec<(&str, bool)> {
        chips.iter().map(|c| (c.label.as_str(), c.tracked)).collect()
    }

    #[test]
    fn compact_labels_fold_a_tracked_branch_and_shorten_names() {
        let main = branch("refs/heads/main", Some("refs/remotes/origin/main"), true);
        let tracked = branch("refs/remotes/origin/main", None, false);
        let folder = branch("refs/remotes/origin/rocket-ui-test/commands", None, false);
        let here = [&main, &tracked, &folder];
        let origin = [remote("origin")];
        assert_eq!(
            labels(&branch_chips(&here, &origin, false)),
            [("main", false), ("origin/main", false), ("origin/rocket-ui-test/commands", false)],
            "the default style is unchanged",
        );
        let compact = branch_chips(&here, &origin, true);
        assert_eq!(labels(&compact), [("main", true), ("r/commands", false)]);
        assert_eq!(compact[0].full, "main · origin/main");
        assert_eq!(compact[1].full, "origin/rocket-ui-test/commands");
        assert!(compact[0].head && !compact[0].remote && compact[1].remote);
        // Two remotes keep the remote's name, and only the tracked one folds in.
        let other = branch("refs/remotes/upstream/main", None, false);
        let two = [remote("origin"), remote("upstream")];
        assert_eq!(
            labels(&branch_chips(&[&main, &tracked, &other, &folder], &two, true)),
            [("main", true), ("upstream/main", false), ("origin/r/commands", false)],
        );
        // A remote branch tracked from another commit keeps its own chip elsewhere.
        assert_eq!(labels(&branch_chips(&[&main], &origin, true)), [("main", false)]);
        let nested = branch("refs/heads/feature/login/form", None, false);
        assert_eq!(labels(&branch_chips(&[&nested], &origin, true)), [("f/l/form", false)]);
        let unusual = branch("refs/heads/文档/说明", None, false);
        assert_eq!(labels(&branch_chips(&[&unusual], &origin, true)), [("文/说明", false)]);
    }

    #[test]
    fn recent_paths_display_without_the_extended_length_prefix() {
        assert_eq!(
            display_path(Path::new(r"\\?\C:\Users\Dev\仓库 🚀")),
            r"C:\Users\Dev\仓库 🚀"
        );
        assert_eq!(
            display_path(Path::new(r"\\?\UNC\server\share\repo")),
            r"\\server\share\repo"
        );
        assert_eq!(display_path(Path::new(r"D:\repo")), r"D:\repo");
        assert_eq!(
            display_path(Path::new(r"\\server\share\repo")),
            r"\\server\share\repo"
        );
    }

    #[test]
    fn commit_draft_splits_into_summary_and_description_and_round_trips() {
        for draft in [
            "",
            "Summary only",
            "Summary\n\nBody line\n\nSecond paragraph 中文",
            "Summary\n\n\nIndented body",
        ] {
            let (summary, description) = split_draft(draft);
            assert!(!summary.contains('\n'));
            assert_eq!(join_draft(&summary, &description), draft);
        }
        assert_eq!(split_draft("A\nB"), ("A".into(), "B".into()));
        assert_eq!(split_draft("A\r\n\r\nB"), ("A".into(), "B".into()));
        assert_eq!(join_draft("A", ""), "A");
    }
}
