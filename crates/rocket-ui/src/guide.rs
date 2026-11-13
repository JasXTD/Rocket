//! Help → Guide: how Rocket's operations work, by topic.
//!
//! Dialogs show what a decision needs: the preview, warnings about this action, errors
//! and results. The background (what is checked again, what is kept for recovery, what
//! cancelling means) lives in `guide.md`. F1, Help → Guide or the "?" in a review
//! dialog's header opens the topic for what is on screen. The guide is its own modal
//! drawn after the dialogs, so the dialog underneath keeps its state.
use crate::{
    app::{DialogKind, Rocket, View},
    backend::Action,
    pages,
    style::*,
};
use eframe::egui::*;
use std::sync::OnceLock;

/// Guide topics in sidebar order. Each title is a `## ` heading in `guide.md`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Topic {
    GettingStarted,
    Safety,
    Repositories,
    Explore,
    Commits,
    Branches,
    Remotes,
    FetchPull,
    Push,
    ForcePush,
    AtomicPush,
    Merge,
    Rebase,
    Conflicts,
    Stashes,
    Recovery,
    Tags,
    Worktrees,
    Agents,
    Submodules,
    Lfs,
    Patches,
    Files,
    Ignore,
    Bisect,
    FasterHistory,
    GitFlow,
    Signing,
    CustomCommands,
    IssueLinks,
    Hosting,
    CiChecks,
    Insights,
    Integrations,
    SshKeys,
    FileHistory,
    Activity,
    Preferences,
    Themes,
    CommitSwitch,
}

impl Topic {
    pub(crate) const ALL: [Topic; 40] = [
        Topic::GettingStarted,
        Topic::Safety,
        Topic::Repositories,
        Topic::Explore,
        Topic::Commits,
        Topic::Branches,
        Topic::Remotes,
        Topic::FetchPull,
        Topic::Push,
        Topic::ForcePush,
        Topic::AtomicPush,
        Topic::Merge,
        Topic::Rebase,
        Topic::Conflicts,
        Topic::Stashes,
        Topic::Recovery,
        Topic::Tags,
        Topic::Worktrees,
        Topic::Agents,
        Topic::Submodules,
        Topic::Lfs,
        Topic::Patches,
        Topic::Files,
        Topic::Ignore,
        Topic::Bisect,
        Topic::FasterHistory,
        Topic::GitFlow,
        Topic::Signing,
        Topic::CustomCommands,
        Topic::IssueLinks,
        Topic::Hosting,
        Topic::CiChecks,
        Topic::Insights,
        Topic::SshKeys,
        Topic::FileHistory,
        Topic::Activity,
        Topic::Preferences,
        Topic::Themes,
        Topic::CommitSwitch,
        Topic::Integrations,
    ];

    pub(crate) fn title(self) -> &'static str {
        match self {
            Topic::GettingStarted => crate::i18n::mark("Getting started"),
            Topic::Safety => crate::i18n::mark("Reviews and safety"),
            Topic::Repositories => crate::i18n::mark("Repositories"),
            Topic::Explore => crate::i18n::mark("Exploring other repositories"),
            Topic::Commits => crate::i18n::mark("Commits"),
            Topic::Branches => crate::i18n::mark("Branches"),
            Topic::Remotes => crate::i18n::mark("Remotes"),
            Topic::FetchPull => crate::i18n::mark("Fetch and pull"),
            Topic::Push => crate::i18n::mark("Push"),
            Topic::ForcePush => crate::i18n::mark("Force push with lease"),
            Topic::AtomicPush => crate::i18n::mark("Push branches together"),
            Topic::Merge => crate::i18n::mark("Merge"),
            Topic::Rebase => crate::i18n::mark("Rebase"),
            Topic::Conflicts => crate::i18n::mark("Conflicts"),
            Topic::Stashes => crate::i18n::mark("Stashes"),
            Topic::Recovery => crate::i18n::mark("Reflog and recovery"),
            Topic::Tags => crate::i18n::mark("Tags"),
            Topic::Worktrees => crate::i18n::mark("Worktrees"),
            Topic::Agents => crate::i18n::mark("Agents"),
            Topic::Submodules => crate::i18n::mark("Submodules"),
            Topic::Lfs => crate::i18n::mark("Git LFS"),
            Topic::Patches => crate::i18n::mark("Patches"),
            Topic::Files => crate::i18n::mark("Restoring files"),
            Topic::Ignore => crate::i18n::mark("Ignore rules"),
            Topic::Bisect => crate::i18n::mark("Bisect"),
            Topic::FasterHistory => crate::i18n::mark("Faster history"),
            Topic::GitFlow => crate::i18n::mark("Git Flow"),
            Topic::Signing => crate::i18n::mark("Signing"),
            Topic::CustomCommands => crate::i18n::mark("Custom commands"),
            Topic::IssueLinks => crate::i18n::mark("Issue links"),
            Topic::Hosting => crate::i18n::mark("Hosting and pull requests"),
            Topic::CiChecks => crate::i18n::mark("CI checks"),
            Topic::Insights => crate::i18n::mark("Insights"),
            Topic::Integrations => crate::i18n::mark("Extensions"),
            Topic::SshKeys => crate::i18n::mark("SSH keys"),
            Topic::FileHistory => crate::i18n::mark("File history"),
            Topic::Activity => crate::i18n::mark("Activity and diagnostics"),
            Topic::Preferences => crate::i18n::mark("Preferences and tools"),
            Topic::Themes => crate::i18n::mark("Themes"),
            Topic::CommitSwitch => crate::i18n::mark("Commit & Push and saved switching"),
        }
    }
}

enum Block {
    Heading(String),
    Paragraph(String),
    Bullet(String),
}

/// `guide.md` split into titled sections. `## ` starts a section, `### ` adds a
/// subheading and `- ` a bullet; other lines continue the paragraph or bullet above
/// them, and a blank line ends it. HTML comments are notes for editors.
fn parse(source: &str) -> Vec<(String, Vec<Block>)> {
    let mut sections: Vec<(String, Vec<Block>)> = Vec::new();
    let (mut open, mut comment) = (false, false);
    for line in source.lines().map(str::trim) {
        if comment || line.starts_with("<!--") {
            comment = !line.contains("-->");
            continue;
        }
        if let Some(title) = line.strip_prefix("## ") {
            sections.push((title.trim().into(), Vec::new()));
            open = false;
            continue;
        }
        // The document title and anything else before the first section is not shown.
        let Some((_, blocks)) = sections.last_mut() else {
            continue;
        };
        if let Some(text) = line.strip_prefix("### ") {
            blocks.push(Block::Heading(text.trim().into()));
            open = false;
        } else if let Some(text) = line.strip_prefix("- ") {
            blocks.push(Block::Bullet(text.trim().into()));
            open = true;
        } else if line.is_empty() {
            open = false;
        } else if open && let Some(Block::Paragraph(text) | Block::Bullet(text)) = blocks.last_mut()
        {
            text.push(' ');
            text.push_str(line);
        } else {
            blocks.push(Block::Paragraph(line.into()));
            open = true;
        }
    }
    sections
}

fn sections() -> &'static [(String, Vec<Block>)] {
    static SECTIONS: OnceLock<Vec<(String, Vec<Block>)>> = OnceLock::new();
    SECTIONS.get_or_init(|| parse(include_str!("guide.md")))
}

/// This frame's topic for what is on screen; a review header shows "?" only with one.
fn context_id() -> Id {
    Id::unique("rocket-guide-context")
}

/// Set by a header "?" or Help → Guide; the guide opens after this frame's dialogs.
fn request_id() -> Id {
    Id::unique("rocket-guide-request")
}

/// Open the guide at the topic for what is on screen, once this frame's dialogs are drawn.
pub(crate) fn request(ctx: &Context) {
    ctx.data_mut(|d| d.insert_temp(request_id(), true));
}

/// Whether the dialog on screen has a guide topic, so its header shows "?".
pub(crate) fn available(ctx: &Context) -> bool {
    ctx.data(|d| d.get_temp::<Option<Topic>>(context_id()))
        .flatten()
        .is_some()
}

/// The "?" at the right of a review dialog's header.
pub(crate) fn header_button(ui: &mut Ui) {
    let (rect, response) = ui.allocate_exact_size(vec2(26.0, 26.0), Sense::click());
    let hovered = response.hovered() || response.has_focus();
    if hovered {
        ui.painter().circle_filled(rect.center(), 13.0, HOVER());
    }
    ui.painter().circle_stroke(
        rect.center(),
        10.5,
        Stroke::new(1.0, if hovered { MUTED() } else { BORDER() }),
    );
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        "?",
        semibold(13.0),
        if hovered { TEXT() } else { MUTED() },
    );
    response.widget_info(|| WidgetInfo::labeled(Role::Button, true, crate::i18n::tr("Guide")));
    if response.on_hover_text(crate::i18n::tr("Guide (F1)")).clicked() {
        request(ui.ctx());
    }
}

impl Rocket {
    /// Before dialogs draw: record this frame's topic for the "?" buttons, open the
    /// guide on F1, and while it is open take Esc and Ctrl+Enter, so the dialog
    /// underneath neither closes nor submits.
    pub(crate) fn guide_input(&mut self, ctx: &Context) {
        let topic = self.guide_topic();
        ctx.data_mut(|d| d.insert_temp(context_id(), topic));
        if self.guide.is_some() {
            let (escape, _) = ctx.input_mut(|i| {
                (
                    i.consume_key(Modifiers::NONE, Key::Escape),
                    i.consume_key(crate::shortcuts::primary_modifiers(), Key::Enter),
                )
            });
            if escape {
                self.guide = None;
            }
        } else if !self.terminal_focused(ctx)
            && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::F1))
        {
            self.guide = Some(topic.unwrap_or(Topic::GettingStarted));
        }
    }

    /// After dialogs, so the guide is the topmost modal.
    pub(crate) fn guide_ui(&mut self, ctx: &Context) {
        let requested = ctx
            .data_mut(|d| d.remove_temp::<bool>(request_id()))
            .unwrap_or(false);
        if requested && self.guide.is_none() {
            self.guide = Some(self.guide_topic().unwrap_or(Topic::GettingStarted));
        }
        let Some(mut topic) = self.guide else {
            return;
        };
        let content = ctx.content_rect();
        let width = (content.width() - 48.0).clamp(300.0, 860.0);
        let height = (content.height() - 190.0).clamp(160.0, 600.0);
        let list = (width * 0.28).clamp(140.0, 220.0);
        let mut close = false;
        let response = Modal::new(Id::unique("rocket-guide"))
            .backdrop_color(Color32::from_black_alpha(120))
            .frame(pages::modal_frame())
            .show(ctx, |ui| {
                ui.set_width(width);
                ui.spacing_mut().item_spacing = Vec2::ZERO;
                Frame::new()
                    .inner_margin(Margin {
                        left: 22,
                        right: 22,
                        top: 18,
                        bottom: 14,
                    })
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            let (badge, _) =
                                ui.allocate_exact_size(vec2(38.0, 38.0), Sense::hover());
                            ui.painter().circle_filled(
                                badge.center(),
                                19.0,
                                BLUE().gamma_multiply(0.15),
                            );
                            ui.painter().text(
                                badge.center(),
                                Align2::CENTER_CENTER,
                                "?",
                                semibold(18.0),
                                BLUE(),
                            );
                            ui.add_space(10.0);
                            ui.vertical(|ui| {
                                ui.spacing_mut().item_spacing.y = 3.0;
                                ui.label(RichText::new(crate::i18n::tr("Guide")).font(semibold(16.0)));
                                ui.label(
                                    RichText::new(crate::i18n::tr("How Rocket's operations work, by topic."))
                                        .color(MUTED()),
                                );
                            });
                            ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                                close |= crate::chrome::icon_button(ui, Icon::Close, true, false, 28.0)
                                    .on_hover_text(crate::i18n::tr("Close (Esc)"))
                                    .clicked();
                            });
                        });
                    });
                pages::divider(ui);
                ui.horizontal_top(|ui| {
                    ui.allocate_ui_with_layout(
                        vec2(list, height),
                        Layout::top_down(Align::Min),
                        |ui| {
                            ui.set_min_size(vec2(list, height));
                            ScrollArea::vertical()
                                .id_salt("guide-topics")
                                .auto_shrink([false, false])
                                .show(ui, |ui| {
                                    Frame::new().inner_margin(Margin::symmetric(10, 8)).show(
                                        ui,
                                        |ui| {
                                            ui.spacing_mut().item_spacing.y = 1.0;
                                            for t in Topic::ALL {
                                                if topic_row(ui, crate::i18n::tr(t.title()), t == topic).clicked() {
                                                    topic = t;
                                                }
                                            }
                                        },
                                    );
                                });
                        },
                    );
                    let (line, _) = ui.allocate_exact_size(vec2(1.0, height), Sense::hover());
                    ui.painter()
                        .vline(line.center().x, line.y_range(), Stroke::new(1.0, HAIRLINE()));
                    let rest = ui.available_width();
                    ui.allocate_ui_with_layout(
                        vec2(rest, height),
                        Layout::top_down(Align::Min),
                        |ui| {
                            ui.set_min_size(vec2(rest, height));
                            // Each topic keeps its own scroll position.
                            ScrollArea::vertical()
                                .id_salt(("guide-topic", topic.title()))
                                .auto_shrink([false, false])
                                .show(ui, |ui| {
                                    Frame::new()
                                        .inner_margin(Margin {
                                            left: 22,
                                            right: 22,
                                            top: 14,
                                            bottom: 18,
                                        })
                                        .show(ui, |ui| {
                                            ui.set_width(ui.available_width());
                                            topic_body(ui, topic);
                                        });
                                });
                        },
                    );
                });
                // No footer: the header's × and Esc close the guide.
            });
        // Esc was taken before the dialogs drew; this is a click outside the guide.
        close |= response.should_close();
        self.guide = (!close).then_some(topic);
    }

    /// The topic for the dialog or page on screen, or else for the current view.
    fn guide_topic(&self) -> Option<Topic> {
        // A page that belongs to another repository tab is not on screen.
        let dialog = self
            .dialog
            .as_ref()
            .filter(|d| pages::Page::of(&d.kind).is_none() || self.page().is_some());
        let Some(d) = dialog else {
            if self.ci.rerun.is_some() {
                return Some(Topic::CiChecks);
            }
            if self.external.open || self.picker.is_some() {
                return Some(Topic::Preferences);
            }
            if self.activity_history.open {
                return Some(Topic::Activity);
            }
            return Some(match self.tab().map(|t| t.view) {
                Some(View::Changes) => Topic::Commits,
                Some(View::FileHistory) => Topic::FileHistory,
                Some(View::Conflict) => Topic::Conflicts,
                Some(View::Branches) => Topic::Branches,
                Some(View::Remotes) => Topic::Remotes,
                Some(View::Stashes) => Topic::Stashes,
                Some(View::Actions) => Topic::CiChecks,
                Some(View::Agents) => Topic::Agents,
                Some(View::Summary | View::Commits | View::Tree | View::Compare | View::BranchReview | View::Map) | None => Topic::GettingStarted,
            });
        };
        Some(match &d.kind {
            DialogKind::Explore | DialogKind::KeepPreview(_) => Topic::Explore,
            DialogKind::RepositoryManager
            | DialogKind::Open
            | DialogKind::NewRepository
            | DialogKind::Clone
            | DialogKind::CloseTab(_) => Topic::Repositories,
            DialogKind::RepositorySettings if d.issue_links.as_ref().is_some_and(|e| e.shown) => {
                Topic::IssueLinks
            }
            DialogKind::RepositorySettings => Topic::Repositories,
            DialogKind::Signing(_) => Topic::Signing,
            DialogKind::NewBranch | DialogKind::TrackBranch { .. } | DialogKind::Rename(_) | DialogKind::DeleteBranches => {
                Topic::Branches
            }
            DialogKind::RunCommand(_) => Topic::CustomCommands,
            DialogKind::AddRemote | DialogKind::EditRemote(..) => Topic::Remotes,
            DialogKind::Fetch | DialogKind::Pull => Topic::FetchPull,
            DialogKind::Push if d.force_push.is_some() => Topic::ForcePush,
            DialogKind::Push => Topic::Push,
            DialogKind::AtomicPush => Topic::AtomicPush,
            DialogKind::Ignore(_) => Topic::Ignore,
            DialogKind::Workflows => match d.workflow.as_ref().map(|s|s.mode) {Some(16|17)=>Topic::CommitSwitch,Some(2..=4|10|11|13|15)=>Topic::Stashes,Some(14)=>Topic::Ignore,Some(5)=>Topic::Lfs,_=>Topic::Remotes},
            DialogKind::Stash => Topic::Stashes,
            DialogKind::Merge => Topic::Merge,
            DialogKind::CommitAction(_) | DialogKind::DraftNotice(_) => Topic::Commits,
            DialogKind::Rebase(_) => Topic::Rebase,
            DialogKind::Bisect => Topic::Bisect,
            DialogKind::HistoryIndex => Topic::FasterHistory,
            DialogKind::Recovery | DialogKind::UndoCommit => Topic::Recovery,
            DialogKind::Tags => Topic::Tags,
            DialogKind::Worktrees => Topic::Worktrees,
            DialogKind::Submodules => Topic::Submodules,
            DialogKind::Lfs => Topic::Lfs,
            DialogKind::GitHub | DialogKind::Hosting => Topic::Hosting,
            DialogKind::Integrations => Topic::Integrations,
            DialogKind::GitFlow(_) => Topic::GitFlow,
            DialogKind::FileEdit(_) => Topic::Files,
            DialogKind::Patch(_) => Topic::Patches,
            DialogKind::MailImport(_) => Topic::Patches,
            DialogKind::ConflictReload(_) => Topic::Conflicts,
            DialogKind::Confirm(action, _) => match action {
                Action::DeleteBranch(_) => Topic::Branches,
                Action::RemoveRemote(_) => Topic::Remotes,
                Action::AbortMerge | Action::FinishMerge => Topic::Merge,
                Action::DropStash(_) | Action::ApplyStash { .. } => Topic::Stashes,
                Action::SaveConflict { .. } | Action::KeepConflictSide { .. } => Topic::Conflicts,
                _ => return None,
            },
            DialogKind::Preferences => match self.preferences.page {
                crate::preferences::Page::SshKeys => Topic::SshKeys,
                crate::preferences::Page::Accounts => Topic::Hosting,
                crate::preferences::Page::Commands => Topic::CustomCommands,
                crate::preferences::Page::Appearance => Topic::Themes,
                crate::preferences::Page::Integrations => Topic::Integrations,
                _ => Topic::Preferences,
            },
            DialogKind::Statistics => Topic::Insights,
            DialogKind::Benchmark => Topic::Activity,
            DialogKind::About
            | DialogKind::Legal
            | DialogKind::Changelog
            | DialogKind::Quit => {
                return None;
            }
        })
    }
}

/// A topic in the guide's sidebar: text only, with the accent bar when selected.
fn topic_row(ui: &mut Ui, title: &str, selected: bool) -> Response {
    let response = ui.add(
        Button::new("")
            .frame(false)
            .min_size(vec2(ui.available_width(), 28.0)),
    );
    let rect = response.rect;
    row_background(
        ui.painter(),
        rect,
        selected,
        response.hovered() || response.has_focus(),
    );
    if selected {
        ui.painter().rect_filled(
            Rect::from_min_size(rect.min + vec2(0.0, 7.0), vec2(3.0, rect.height() - 14.0)),
            1.5,
            BLUE(),
        );
    }
    let color = if selected { TEXT() } else { MUTED() };
    let mut job =
        text::LayoutJob::simple_singleline(title.into(), FontId::proportional(13.0), color);
    job.wrap = text::TextWrapping::truncate_at_width((rect.width() - 22.0).max(0.0));
    let galley = ui.painter().layout_job(job);
    ui.painter().galley(
        pos2(rect.left() + 12.0, rect.center().y - galley.size().y / 2.0),
        galley,
        color,
    );
    response.widget_info(|| WidgetInfo::selected(Role::Button, true, selected, title));
    response
}

fn topic_body(ui: &mut Ui, topic: Topic) {
    ui.spacing_mut().item_spacing.y = 7.0;
    ui.label(RichText::new(crate::i18n::tr(topic.title())).font(semibold(18.0)));
    let Some((_, blocks)) = sections().iter().find(|(title, _)| title == topic.title()) else {
        return;
    };
    for block in blocks {
        match block {
            Block::Heading(text) => {
                ui.add_space(6.0);
                ui.label(RichText::new(text).font(semibold(13.5)));
            }
            Block::Paragraph(text) => {
                ui.add(Label::new(RichText::new(text).size(13.0)).wrap());
            }
            Block::Bullet(text) => {
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    ui.label(RichText::new("•").size(13.0).color(BLUE()));
                    ui.add(Label::new(RichText::new(text).size(13.0)).wrap());
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_common as common;

    /// The guide opens over a dialog without taking its state: F1 and a header "?"
    /// open the dialog's topic, and while the guide is open Ctrl+Enter cannot submit
    /// the dialog and Esc closes only the guide.
    #[test]
    fn guide_opens_over_a_dialog_and_keeps_its_keys() {
        common::isolated(
            "ROCKET_GUIDE_UI_CASE",
            "guide::tests::guide_opens_over_a_dialog_and_keeps_its_keys",
            || {
                let fixture = common::Fixture::new();
                let ctx = Context::default();
                let mut app = Rocket::with_log_directory(&ctx, vec![], fixture.root.join("logs"));
                let key = |key: Key, modifiers: Modifiers| Event::Key {
                    key,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers,
                };
                let frame = |app: &mut Rocket, events: Vec<Event>| {
                    let input = RawInput {
                        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1200.0, 900.0))),
                        events,
                        ..Default::default()
                    };
                    let mut output = ctx.run_ui(input, |ui| {
                        let ctx = ui.ctx().clone();
                        app.guide_input(&ctx);
                        app.dialog_ui(&ctx);
                        app.guide_ui(&ctx);
                    });
                    output.textures_delta.clear();
                };
                app.show_dialog(DialogKind::Open);
                // Open browses to a typed folder; the fixture repository is one to open.
                let repository = fixture.repository.display().to_string();
                app.dialog.as_mut().unwrap().first = repository;
                frame(&mut app, vec![]);
                assert!(available(&ctx), "the Open dialog offers its topic");

                frame(&mut app, vec![key(Key::F1, Modifiers::NONE)]);
                assert_eq!(app.guide, Some(Topic::Repositories));
                frame(&mut app, vec![key(Key::Enter, crate::shortcuts::primary_modifiers())]);
                assert!(
                    app.dialog.is_some(),
                    "Ctrl+Enter submitted the dialog under the guide"
                );
                frame(&mut app, vec![key(Key::Escape, Modifiers::NONE)]);
                assert_eq!(app.guide, None);
                assert!(
                    app.dialog.is_some(),
                    "Esc closed the dialog under the guide"
                );

                request(&ctx);
                frame(&mut app, vec![]);
                assert_eq!(
                    app.guide,
                    Some(Topic::Repositories),
                    "the header ? opens the topic"
                );
                frame(&mut app, vec![key(Key::Escape, Modifiers::NONE)]);

                // Without the guide, the same Ctrl+Enter submits and closes the dialog
                // once the folder read has found the repository to open.
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
                while app.dialog.as_ref().is_some_and(|d| d.first.is_empty()) {
                    assert!(std::time::Instant::now() < deadline, "the folder read finished");
                    std::thread::sleep(std::time::Duration::from_millis(5));
                    frame(&mut app, vec![]);
                }
                frame(&mut app, vec![key(Key::Enter, crate::shortcuts::primary_modifiers())]);
                assert!(app.dialog.is_none());
            },
        );
    }

    #[test]
    fn every_topic_has_exactly_one_section_in_order() {
        let titles: Vec<&str> = sections().iter().map(|(title, _)| title.as_str()).collect();
        let expected: Vec<&str> = Topic::ALL.iter().map(|t| t.title()).collect();
        assert_eq!(titles, expected, "guide.md sections must match Topic::ALL");
        for (title, blocks) in sections() {
            assert!(!blocks.is_empty(), "{title} has no text");
        }
    }

    #[test]
    fn parser_joins_wrapped_lines_and_skips_comments() {
        let parsed = parse(
            "# Title\nignored\n<!-- a note\nover lines -->\n## One\nFirst line\ncontinues.\n\n- A bullet\n  wraps.\n### Sub\nText\n",
        );
        assert_eq!(parsed.len(), 1);
        let (title, blocks) = &parsed[0];
        assert_eq!(title, "One");
        let texts: Vec<String> = blocks
            .iter()
            .map(|b| match b {
                Block::Heading(t) => format!("h:{t}"),
                Block::Paragraph(t) => format!("p:{t}"),
                Block::Bullet(t) => format!("b:{t}"),
            })
            .collect();
        assert_eq!(
            texts,
            [
                "p:First line continues.",
                "b:A bullet wraps.",
                "h:Sub",
                "p:Text"
            ]
        );
    }
}
