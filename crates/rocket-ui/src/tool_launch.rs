//! The dialog that runs an external tool for one file: choose a profile, launch,
//! wait, then review a merge result. Profiles are edited in Preferences → External
//! tools. Hiding the dialog keeps the tool running; the status bar brings it back.
use crate::{
    app::*,
    backend::user_error,
    external_tools::ToolIntent,
    pages::{self, Part, ReviewHeader},
    preferences::Page,
    style::*,
};
use eframe::egui::*;
use rocket_git::*;

/// Characters of a merge result shown before the review opens the whole file.
const PREVIEW_LIMIT: usize = 12_000;

enum Stage {
    /// `Prepare` failed before anything was pending.
    Refused,
    Ready,
    Running,
    Importing,
    Finished,
}

impl Rocket {
    /// The launched tool's profile name, for buttons and messages.
    fn tool_name(&self) -> String {
        let s = &self.external;
        let group = &s.profiles.0[s.kind.index()];
        group
            .entries
            .get(s.selected[s.kind.index()])
            .unwrap_or(&group.entries[group.default])
            .name
            .clone()
    }

    /// Drawn while `external.open` and no other dialog is up: Edit profiles opens
    /// Preferences on top, and this dialog returns when Preferences closes.
    pub(crate) fn tool_launch_dialog(&mut self, ctx: &Context) {
        if !self.external.open || self.dialog.is_some() || self.quitting {
            return;
        }
        let stage = if self.external.importing() {
            Stage::Importing
        } else if self.external.running() {
            Stage::Running
        } else if self.external.report.is_some() {
            Stage::Finished
        } else if self.external.pending() {
            Stage::Ready
        } else {
            Stage::Refused
        };
        let kind = self.external.kind;
        let name = self.tool_name();
        let (image, title) = match kind {
            ExternalToolKind::Editor => (Icon::Pencil, "Open in editor"),
            ExternalToolKind::Diff => (Icon::Columns, "Open in diff tool"),
            ExternalToolKind::Merge => (Icon::Merge, "Open in merge tool"),
        };
        let subtitle = self.external.label.clone();
        let idle = self.write.is_none() && !self.quitting;
        let unsaved = self.external.profiles != self.external.editing;
        // A merge result waiting for review is kept when the dialog is hidden.
        let reviewable = matches!(stage, Stage::Finished)
            && self
                .external
                .report
                .as_ref()
                .is_some_and(|r| r.result.is_ok() && r.text.is_some());
        let mut intents: Vec<ToolIntent> = Vec::new();
        let mut hide = false;
        let mut dismiss = false;
        let mut edit_profiles = false;
        let widgets = &mut self.frame_widgets;
        let s = &mut self.external;
        let head = ReviewHeader {
            image,
            title,
            subtitle: &subtitle,
            danger: false,
        };
        // A refused launch, or a tool that finished with nothing to review, leaves nothing
        // to decide: no footer, the header's × closes it.
        let quiet = matches!(stage, Stage::Refused) || (matches!(stage, Stage::Finished) && !reviewable);
        let mut content = |ui: &mut Ui, part: Part, widgets: &mut std::collections::HashMap<String, UiControl>| {
            match (part, &stage) {
                (Part::Body, Stage::Refused) => {
                    if let Some(error) = &s.error {
                        pages::callout(ui, RED(), error);
                    }
                }
                (Part::Body, Stage::Ready) => {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Tool").color(MUTED()));
                        ui.add_space(12.0);
                        let layer = ui.layer_id();
                        let group = &s.profiles.0[kind.index()];
                        let selected = &mut s.selected[kind.index()];
                        let current = group
                            .entries
                            .get(*selected)
                            .map_or_else(String::new, |p| p.name.clone());
                        let combo = ComboBox::from_id_salt("tools-launch-profile")
                            .selected_text(current)
                            .width(240.0)
                            .show_ui(ui, |ui| {
                                // Modal and popup share a layer order; keep the list above the dialog.
                                ui.ctx().set_sublayer(layer, ui.layer_id());
                                for (index, profile) in group.entries.iter().enumerate() {
                                    let label = if index == group.default {
                                        format!("{} · default", profile.name)
                                    } else {
                                        profile.name.clone()
                                    };
                                    let r = ui.selectable_label(*selected == index, label);
                                    pages::control(widgets, format!("tools-profile-{index}"), &r, ui.clip_rect());
                                    if r.clicked() {
                                        *selected = index;
                                        ui.close();
                                    }
                                }
                            });
                        pages::control(widgets, "tools-profile-picker", &combo.response, ui.clip_rect());
                        if *selected == group.default {
                            pages::pill(ui, "Default", BLUE());
                        }
                        let edit = ui.add(
                            Button::new(RichText::new("Edit profiles").size(12.5).color(BLUE())).frame(false),
                        );
                        pages::control(widgets, "tools-edit-profiles", &edit, ui.clip_rect());
                        edit_profiles = edit.clicked();
                    });
                    ui.add_space(4.0);
                    let note = match kind {
                        ExternalToolKind::Editor => (
                            AMBER(),
                            format!("{name} edits the real file. Stopping it cannot undo what it saved."),
                        ),
                        ExternalToolKind::Diff => (MUTED(), "Both versions open as read-only copies.".to_owned()),
                        ExternalToolKind::Merge => (
                            MUTED(),
                            format!("{name} edits a temporary copy. You review the result before anything is saved."),
                        ),
                    };
                    pages::callout(ui, note.0, &note.1);
                    if unsaved {
                        pages::callout(ui, AMBER(), "Save or discard the tool profile changes in Preferences first.");
                    } else if s.config().executable.trim().is_empty() {
                        pages::callout(ui, AMBER(), "This profile has no program yet. Choose one in Edit profiles.");
                    }
                    if let Some(error) = &s.error {
                        pages::callout(ui, RED(), error);
                    }
                }
                (Part::Body, Stage::Running | Stage::Importing) => {
                    pages::well(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.vertical(|ui| {
                                let (line, hint) = match &stage {
                                    Stage::Importing => (
                                        "Checking the original file and conflict state".to_owned(),
                                        "The result opens in the conflict editor when the check passes.",
                                    ),
                                    _ if s.cancelling => (format!("Stopping {name}"), "Rocket waits for it and checks what it left."),
                                    _ => (format!("Waiting for {name} to close"), "Save your work there, then close its window."),
                                };
                                ui.label(RichText::new(line).font(semibold(13.0)));
                                ui.label(RichText::new(hint).size(12.0).color(HINT()));
                            });
                        });
                    });
                }
                (Part::Body, Stage::Finished) => {
                    let Some(report) = &s.report else { return };
                    match &report.result {
                        Ok(()) => {
                            ui.horizontal(|ui| {
                                let (rect, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
                                icon(ui.painter(), rect, Icon::Check, GREEN());
                                ui.label(RichText::new(format!("{name} finished")).font(semibold(13.0)));
                            });
                            if let Some(text) = &report.text {
                                pages::caption(ui, "Result preview");
                                pages::well(ui, |ui| {
                                    ScrollArea::both()
                                        .id_salt("tools-result")
                                        .max_height(180.0)
                                        .show(ui, |ui| {
                                            ui.add(
                                                Label::new(
                                                    RichText::new(text.chars().take(PREVIEW_LIMIT).collect::<String>())
                                                        .monospace(),
                                                )
                                                .wrap_mode(TextWrapMode::Extend),
                                            );
                                        });
                                });
                                if text.chars().count() > PREVIEW_LIMIT {
                                    pages::note(ui, "Preview shortened. The review shows the whole result.");
                                }
                            } else if kind == ExternalToolKind::Editor {
                                pages::note(ui, "What it saved is already in the working tree.");
                            }
                        }
                        Err(Error::Cancelled) => {
                            pages::callout(ui, MUTED(), &format!("{name} was stopped. Nothing was imported."));
                        }
                        Err(error) => pages::callout(ui, RED(), &user_error(error)),
                    }
                    if let Some(path) = &report.workspace {
                        pages::caption(ui, "Temporary copies");
                        ui.add(
                            Label::new(RichText::new(path.display().to_string()).size(12.0).monospace().color(HINT()))
                                .selectable(true)
                                .wrap(),
                        );
                    }
                    if let Some(error) = &s.error
                        && report.result.is_ok()
                    {
                        pages::callout(ui, RED(), error);
                    }
                }
                // Not drawn: see `quiet`.
                (Part::Footer, Stage::Refused) => {}
                (Part::Footer, Stage::Ready) => {
                    let ready = idle && !unsaved && !s.config().executable.trim().is_empty();
                    let start = pages::primary(ui, ready, &format!("Launch {name}"));
                    pages::control(widgets, "tools-start", &start, ui.clip_rect());
                    if start.clicked() {
                        intents.push(ToolIntent::Start);
                    }
                }
                (Part::Footer, Stage::Running | Stage::Importing) => {
                    let label = match &stage {
                        Stage::Importing => "Stop the check".to_owned(),
                        _ => format!("Stop {name}"),
                    };
                    let stop = pages::danger_ghost(ui, !s.cancelling, &label);
                    pages::control(widgets, "tools-cancel", &stop, ui.clip_rect());
                    if stop.clicked() {
                        intents.push(ToolIntent::Cancel);
                    }
                    let hide_button = ui.button("Hide").on_hover_text("Keep it running; the status bar shows it");
                    pages::control(widgets, "tools-back", &hide_button, ui.clip_rect());
                    hide = hide_button.clicked();
                }
                (Part::Footer, Stage::Finished) => {
                    if reviewable {
                        let import = pages::primary(ui, idle && !s.imported, "Review in conflict editor");
                        pages::control(widgets, "tools-import", &import, ui.clip_rect());
                        if import.clicked() {
                            intents.push(ToolIntent::Import);
                        }
                        let discard = ui.button("Discard result");
                        pages::control(widgets, "tools-dismiss", &discard, ui.clip_rect());
                        dismiss = discard.clicked();
                        let hide_button = ui.button("Hide").on_hover_text("Keep the result; the status bar shows it");
                        pages::control(widgets, "tools-back", &hide_button, ui.clip_rect());
                        hide = hide_button.clicked();
                    }
                }
            }
        };
        let response = if quiet {
            pages::info(ctx, "rocket-tool", head, widgets, "tools-scroll", &mut content)
        } else {
            pages::review(ctx, "rocket-tool", head, widgets, "tools-scroll", true, &mut content)
        };
        if quiet && let Some(close) = widgets.get("dialog-close").copied() {
            // Automation closed these with their Close button.
            widgets.insert("tools-dismiss".into(), close);
        }
        if matches!(stage, Stage::Ready) && let Some(close) = widgets.get("dialog-close").copied() {
            // Automation cancelled the launch with its Cancel button; while the tool
            // runs, `tools-cancel` stays its Stop.
            widgets.insert("tools-cancel".into(), close);
        }
        if response.should_close() {
            // Esc hides a running tool or a result waiting for review, and otherwise
            // cancels the launch.
            match stage {
                Stage::Running | Stage::Importing => hide = true,
                Stage::Finished if reviewable => hide = true,
                Stage::Ready => {
                    intents.push(ToolIntent::Cancel);
                    dismiss = true;
                }
                Stage::Refused | Stage::Finished => dismiss = true,
            }
        }
        for intent in intents {
            self.external_intent(intent);
        }
        if hide {
            self.external.open = false;
        }
        if dismiss {
            // Temporary workspaces stay on disk for recovery.
            let s = &mut self.external;
            s.report = None;
            s.clear_pending();
            s.label.clear();
            s.error = None;
            s.open = false;
        }
        if edit_profiles {
            self.show_preferences(Page::Tools);
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{app::Rocket, external_tools::ToolIntent, test_common as common};
    use eframe::egui::*;
    use rocket_git::{ExternalToolKind, ExternalToolReport};

    fn frame(ctx: &Context, app: &mut Rocket, events: Vec<Event>) {
        app.frame_widgets.clear();
        let mut output = ctx.run_ui(
            RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 860.0))),
                events,
                ..Default::default()
            },
            |ui| {
                let ctx = ui.ctx().clone();
                app.tool_launch_dialog(&ctx);
            },
        );
        output.textures_delta.clear();
    }

    #[test]
    fn a_refused_launch_says_why_and_closes() {
        common::isolated(
            "ROCKET_TOOL_UI_CASE",
            "tool_launch::tests::a_refused_launch_says_why_and_closes",
            || {
                let fixture = common::Fixture::new();
                let ctx = Context::default();
                let mut app = Rocket::with_log_directory(&ctx, vec![], fixture.root.join("logs"));
                app.external_intent(ToolIntent::Prepare(ExternalToolKind::Diff));
                assert!(app.external.open && app.external.error.is_some());
                // A new modal is placed once its size is known, after one frame.
                frame(&ctx, &mut app, vec![]);
                frame(&ctx, &mut app, vec![]);
                click(&ctx, &mut app, "tools-dismiss");
                assert!(!app.external.open, "Close hides the dialog");
                assert!(app.external.error.is_none());
            },
        );
    }

    /// Press and release the primary button on a control drawn in the last frame.
    fn click(ctx: &Context, app: &mut Rocket, id: &str) {
        let at = app.frame_widgets[id].rect.center();
        let button = |pressed| Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        frame(ctx, app, vec![Event::PointerMoved(at), button(true)]);
        frame(ctx, app, vec![button(false)]);
    }

    #[test]
    fn a_merge_result_hides_with_esc_or_hide_and_only_discard_drops_it() {
        common::isolated(
            "ROCKET_TOOL_UI_CASE",
            "tool_launch::tests::a_merge_result_hides_with_esc_or_hide_and_only_discard_drops_it",
            || {
                let fixture = common::Fixture::new();
                let ctx = Context::default();
                let mut app = Rocket::with_log_directory(&ctx, vec![], fixture.root.join("logs"));
                app.external.kind = ExternalToolKind::Merge;
                app.external.report = Some(ExternalToolReport {
                    workspace: None,
                    preview: None,
                    text: Some("resolved\n".into()),
                    result: Ok(()),
                });
                let show = |app: &mut Rocket| {
                    app.external.open = true;
                    frame(&ctx, app, vec![]);
                    frame(&ctx, app, vec![]);
                    assert!(app.frame_widgets.contains_key("tools-import"), "the result is reviewable");
                };
                show(&mut app);
                click(&ctx, &mut app, "tools-back");
                assert!(!app.external.open && app.external.report.is_some(), "Hide keeps the result");
                show(&mut app);
                let escape = Event::Key {
                    key: Key::Escape,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: Modifiers::NONE,
                };
                frame(&ctx, &mut app, vec![escape]);
                assert!(!app.external.open && app.external.report.is_some(), "Esc keeps the result");
                show(&mut app);
                click(&ctx, &mut app, "tools-dismiss");
                assert!(!app.external.open && app.external.report.is_none(), "Discard drops it");
            },
        );
    }
}
