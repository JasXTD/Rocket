//! Repository management pages (Tags, Worktrees, Submodules, Git LFS, GitHub).
//!
//! Browsing happens on a page in the content area; every decision (create, remove,
//! fetch, unlock…) opens a review dialog on top of it. State still lives in
//! `Rocket::dialog`, so owner checks, MCP state and write completion are unchanged.
//! Every other modal dialog (menus, commit operations, rebase, recovery, file
//! restore and patches) uses the same review layout.
use crate::{app::*, style::*, submodules::ManagementMode};
use eframe::egui::*;
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    RepositoryManager,
    Explore,
    Tags,
    Worktrees,
    Submodules,
    Lfs,
    GitHub,
    Hosting,
    Integrations,
    Statistics,
    Benchmark,
}

impl Page {
    pub fn dialog_kind(self) -> DialogKind {
        match self {
            Self::RepositoryManager => DialogKind::RepositoryManager,
            Self::Explore => DialogKind::Explore,
            Self::Tags => DialogKind::Tags,
            Self::Worktrees => DialogKind::Worktrees,
            Self::Submodules => DialogKind::Submodules,
            Self::Lfs => DialogKind::Lfs,
            Self::GitHub => DialogKind::GitHub,
            Self::Hosting => DialogKind::Hosting,
            Self::Integrations => DialogKind::Integrations,
            Self::Statistics => DialogKind::Statistics,
            Self::Benchmark => DialogKind::Benchmark,
        }
    }

    pub fn of(kind: &DialogKind) -> Option<Self> {
        match kind {
            DialogKind::RepositoryManager => Some(Self::RepositoryManager),
            DialogKind::Explore => Some(Self::Explore),
            DialogKind::Tags => Some(Self::Tags),
            DialogKind::Worktrees => Some(Self::Worktrees),
            DialogKind::Submodules => Some(Self::Submodules),
            DialogKind::Lfs => Some(Self::Lfs),
            DialogKind::GitHub => Some(Self::GitHub),
            DialogKind::Hosting => Some(Self::Hosting),
            DialogKind::Integrations => Some(Self::Integrations),
            DialogKind::Statistics => Some(Self::Statistics),
            DialogKind::Benchmark => Some(Self::Benchmark),
            _ => None,
        }
    }
}

/// A page dialog that is only browsing: no review, form or write in progress.
pub fn browsing(d: &Dialog) -> bool {
    match Page::of(&d.kind) {
        None => false,
        Some(Page::Tags) => d.tags.as_ref().is_none_or(|s| !s.reviewing()),
        Some(Page::Worktrees) => d
            .worktrees
            .as_ref()
            .is_none_or(|s| !s.creating && !s.removing && !s.applying),
        Some(Page::Submodules) => d
            .submodules
            .as_ref()
            .is_none_or(|s| !s.reviewing && s.mode == ManagementMode::Browse && !s.applying),
        Some(Page::Lfs) => d.lfs.as_ref().is_none_or(|s| {
            !s.applying
                && if s.locks_tab {
                    s.locks.action.is_none()
                } else {
                    s.action.is_none()
                }
        }),
        // Read-only: the GitHub page has no review or write.
        Some(Page::GitHub) => true,
        Some(Page::Hosting) => !d.flag,
        Some(Page::Integrations) => true,
        Some(Page::RepositoryManager) => !d.flag,
        // Explore never reviews or writes; Preview opens a tab.
        Some(Page::Explore) => true,
        // Read-only.
        Some(Page::Statistics) => true,
        Some(Page::Benchmark) => !d.flag,
    }
}

impl Rocket {
    /// The management page shown for the active repository, if any.
    pub fn page(&self) -> Option<Page> {
        let d = self.dialog.as_ref()?;
        let page = Page::of(&d.kind)?;
        let active = self.tab().map(|t| t.id);
        // Hosting and Integrations also work from the welcome screen. Repository-owned
        // pages still require the original tab; a review never follows another tab.
        (matches!(page, Page::RepositoryManager | Page::Explore)
            || (d.owner == active
                && (d.owner.is_some() || matches!(page, Page::Hosting | Page::Integrations))))
            .then_some(page)
    }

    /// A dialog that owns input: any modal dialog, or a page with a review or
    /// write in progress. Browsing pages leave shortcuts and navigation available.
    pub fn blocking_dialog(&self) -> bool {
        self.agents.form.is_some() || self.blocking_dialog_except_agents()
    }

    /// The Agents modal may use its own controls, never those behind another modal.
    pub fn blocking_dialog_except_agents(&self) -> bool {
        self.crash_reports.open || self.dependency_push_review_open() || self.updates.open || self.commit_link.is_some() || self.ci.rerun.is_some() || self.dialog.as_ref().is_some_and(|d| !browsing(d))
            || self.picker.is_some()
            || self.integrations.prompt_open()
            || self.external.open
            || self.map_export_open()
    }

    /// Leave a browsing page, e.g. when the user navigates to another view.
    pub(crate) fn close_browsing_page(&mut self) {
        if self.dialog.as_ref().is_some_and(browsing) {
            self.dialog = None;
        }
    }

    pub(crate) fn page_ui(&mut self, ui: &mut Ui, page: Page) {
        match page {
            Page::RepositoryManager => self.repository_manager_page(ui),
            Page::Tags => self.tag_page(ui),
            Page::Worktrees => self.worktree_page(ui),
            Page::Submodules => self.submodule_page(ui),
            Page::Lfs => self.lfs_page(ui),
            Page::GitHub => self.github_page(ui),
            Page::Hosting => self.forge_page(ui),
            Page::Integrations => self.integration_page(ui),
            Page::Statistics => self.statistics_page(ui),
            Page::Benchmark => self.benchmark_page(ui),
            Page::Explore => self.explore_page(ui),
        }
    }
}

/// Publish a control's visible part for automation.
pub(crate) fn control(
    widgets: &mut HashMap<String, UiControl>,
    id: impl Into<String>,
    r: &Response,
    clip: Rect,
) {
    let rect = r.rect.intersect(clip);
    if rect.is_positive() {
        widgets.insert(
            id.into(),
            UiControl {
                rect,
                enabled: r.enabled(),
            },
        );
    }
}

pub(crate) fn region(
    widgets: &mut HashMap<String, UiControl>,
    id: &str,
    rect: Rect,
    enabled: bool,
) {
    if rect.is_positive() {
        widgets.insert(id.into(), UiControl { rect, enabled });
    }
}

/// Where the last [`split`] drew its details card's scroll viewport this frame.
fn details_viewport_id() -> Id {
    Id::new("page-details-viewport")
}

/// Publish the details card's visible viewport as `page-details-scroll`, so automation
/// can scroll to card actions (Delete, Checkout…) when a short window clips the card.
/// Call right after [`split`]; only a page that is browsing passes its live widgets.
pub(crate) fn details_scroll(ui: &Ui, widgets: &mut HashMap<String, UiControl>, enabled: bool) {
    let frame = ui.ctx().cumulative_frame_nr();
    // Other views also use `split`; only a viewport drawn in this frame counts.
    if let Some((drawn, rect)) = ui
        .ctx()
        .data(|d| d.get_temp::<(u64, Rect)>(details_viewport_id()))
        && drawn == frame
    {
        region(widgets, "page-details-scroll", rect, enabled);
    }
}

/// Page title: icon tile, title and description on the left, actions on the right.
pub(crate) fn header(
    ui: &mut Ui,
    image: Icon,
    title: &str,
    description: &str,
    actions: impl FnOnce(&mut Ui),
) {
    header_with(ui, |painter, tile| icon(painter, tile.shrink(11.0), image, BLUE()), title, description, actions);
}

/// A page title whose tile shows what `tile` paints on it.
pub(crate) fn header_with(
    ui: &mut Ui,
    tile: impl FnOnce(&Painter, Rect),
    title: &str,
    description: &str,
    actions: impl FnOnce(&mut Ui),
) {
    Frame::new()
        .inner_margin(Margin {
            left: 24,
            right: 20,
            top: 18,
            bottom: 14,
        })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                // Actions are laid out first from the right; the title takes the rest.
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    actions(ui);
                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                        let (rect, _) = ui.allocate_exact_size(vec2(40.0, 40.0), Sense::hover());
                        ui.painter().rect_filled(rect, 10.0, RAISED());
                        tile(ui.painter(), rect);
                        ui.add_space(6.0);
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 2.0;
                            ui.label(RichText::new(title).font(semibold(18.0)));
                            ui.add(
                                Label::new(RichText::new(description).size(12.5).color(MUTED()))
                                    .truncate(),
                            );
                        });
                    });
                });
            });
        });
}

pub(crate) struct TabItem<'a> {
    pub label: &'a str,
    pub count: Option<usize>,
    pub selected: bool,
    pub enabled: bool,
    pub key: &'a str,
}

/// Underlined page tabs. Returns the index of a clicked, enabled tab.
pub(crate) fn tabs(
    ui: &mut Ui,
    items: &[TabItem],
    widgets: &mut HashMap<String, UiControl>,
) -> Option<usize> {
    let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), 38.0), Sense::hover());
    ui.painter().hline(
        row.x_range(),
        row.bottom() - 0.5,
        Stroke::new(1.0, HAIRLINE()),
    );
    let mut x = row.left() + 24.0;
    let mut clicked = None;
    for (i, item) in items.iter().enumerate() {
        let font = if item.selected {
            semibold(13.0)
        } else {
            FontId::proportional(13.0)
        };
        let galley = ui
            .painter()
            .layout_no_wrap(item.label.into(), font.clone(), TEXT());
        let count = item.count.map(|n| n.to_string());
        let count_width = count.as_ref().map_or(0.0, |c| c.len() as f32 * 7.0 + 18.0);
        let rect = Rect::from_min_size(
            pos2(x, row.top()),
            vec2(galley.size().x + count_width + 4.0, row.height()),
        );
        let response = ui.interact(
            rect,
            ui.id().with(("page-tab", item.key)),
            if item.enabled {
                Sense::click()
            } else {
                Sense::hover()
            },
        );
        let color = if !item.enabled {
            FAINT()
        } else if item.selected || response.hovered() {
            TEXT()
        } else {
            MUTED()
        };
        ui.painter().galley(
            pos2(rect.left(), rect.center().y - galley.size().y / 2.0),
            galley.clone(),
            color,
        );
        if let Some(count) = count {
            count_pill(
                ui.painter(),
                pos2(
                    rect.left() + galley.size().x + 6.0 + count_width / 2.0,
                    rect.center().y,
                ),
                &count,
                FAINT(),
            );
        }
        if item.selected {
            ui.painter().rect_filled(
                Rect::from_min_max(
                    pos2(rect.left(), rect.bottom() - 2.0),
                    pos2(rect.right(), rect.bottom()),
                ),
                1.0,
                BLUE(),
            );
        }
        response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, item.enabled, item.label));
        let control_rect = rect.intersect(ui.clip_rect());
        if control_rect.is_positive() {
            widgets.insert(
                item.key.into(),
                UiControl {
                    rect: control_rect,
                    enabled: item.enabled,
                },
            );
        }
        if response.clicked() && item.enabled {
            clicked = Some(i);
        }
        x = rect.right() + 24.0;
    }
    clicked
}

/// The row of page controls under the title or tabs.
pub(crate) fn toolbar(ui: &mut Ui, content: impl FnOnce(&mut Ui)) {
    Frame::new()
        .inner_margin(Margin {
            left: 24,
            right: 24,
            top: 12,
            bottom: 8,
        })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(8.0, 6.0);
                content(ui)
            });
        });
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Side {
    List,
    Details,
}

/// List on the left and a details card on the right; stacked in narrow panes.
/// One closure renders both sides so they can share mutable page state.
pub(crate) fn split(ui: &mut Ui, mut side: impl FnMut(&mut Ui, Side)) {
    let width = ui.available_width() - 48.0;
    let height = (ui.available_height() - 16.0).max(80.0);
    Frame::new()
        .inner_margin(Margin {
            left: 24,
            right: 24,
            top: 4,
            bottom: 12,
        })
        .show(ui, |ui| {
            if width >= 700.0 {
                let details = (width * 0.4).clamp(300.0, 440.0);
                // The 16-point gap comes on top of the layout's item spacing.
                let list = width - details - 16.0 - ui.spacing().item_spacing.x;
                ui.horizontal_top(|ui| {
                    ui.allocate_ui_with_layout(
                        vec2(list, height),
                        Layout::top_down(Align::Min),
                        |ui| {
                            ui.set_min_size(vec2(list, height));
                            side(ui, Side::List);
                        },
                    );
                    ui.add_space(16.0);
                    ui.allocate_ui_with_layout(
                        vec2(details, height),
                        Layout::top_down(Align::Min),
                        |ui| {
                            ui.set_min_size(vec2(details, height));
                            card(ui, |ui| side(ui, Side::Details));
                        },
                    );
                });
            } else {
                ui.set_width(width);
                ui.allocate_ui_with_layout(
                    vec2(width, (height * 0.5).max(120.0)),
                    Layout::top_down(Align::Min),
                    |ui| side(ui, Side::List),
                );
                ui.add_space(12.0);
                card(ui, |ui| side(ui, Side::Details));
            }
        });
}

/// Details card: a raised surface with its own scroll area.
fn card(ui: &mut Ui, content: impl FnOnce(&mut Ui)) {
    Frame::new()
        .fill(PANEL())
        .stroke(Stroke::new(1.0, HAIRLINE()))
        .corner_radius(10)
        .inner_margin(16)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            // `split` keeps a minimum height, so an open drawer or a long notice can push
            // the card's lower part off screen. Size the viewport to what is shown (below
            // egui's 64 px minimum if need be) so the last action can still scroll into view.
            let shown = (ui.clip_rect().bottom() - ui.available_rect_before_wrap().top()).max(0.0);
            let output = ScrollArea::vertical()
                .id_salt(ui.id().with("page-card"))
                .max_height(shown)
                .min_scrolled_height(shown.min(64.0))
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 6.0;
                    content(ui)
                });
            let viewport = output.inner_rect.intersect(ui.clip_rect());
            let frame = ui.ctx().cumulative_frame_nr();
            ui.ctx()
                .data_mut(|d| d.insert_temp(details_viewport_id(), (frame, viewport)));
        });
}

pub(crate) struct Row<'a> {
    pub image: Icon,
    pub title: &'a str,
    pub detail: &'a str,
    pub pills: Vec<(String, Color32)>,
    pub indent: f32,
}

/// A 46-point list row: icon tile, title, detail line and status pills.
pub(crate) fn row(ui: &mut Ui, row: Row, selected: bool, enabled: bool) -> Response {
    let response = ui.add_enabled(
        enabled,
        Button::new("")
            .frame(false)
            .min_size(vec2(ui.available_width(), 46.0)),
    );
    let rect = response.rect;
    let painter = ui.painter();
    if selected {
        painter.rect_filled(rect, 8.0, SELECTED());
        painter.rect_filled(
            Rect::from_min_size(rect.min + vec2(0.0, 10.0), vec2(3.0, rect.height() - 20.0)),
            1.5,
            BLUE(),
        );
    } else if enabled && (response.hovered() || response.has_focus()) {
        painter.rect_filled(rect, 8.0, HOVER());
    }
    let left = rect.left() + 10.0 + row.indent;
    let tile = Rect::from_min_size(pos2(left, rect.center().y - 15.0), vec2(30.0, 30.0));
    painter.rect_filled(tile, 7.0, RAISED());
    icon(
        painter,
        tile.shrink(8.0),
        row.image,
        if selected { BLUE() } else { MUTED() },
    );
    // Pills sit on the right; the text column is clipped before them.
    let mut right = rect.right() - 10.0;
    for (text, color) in row.pills.iter().rev() {
        let galley = painter.layout_no_wrap(text.clone(), semibold(11.0), *color);
        let pill = Rect::from_min_max(
            pos2(right - galley.size().x - 16.0, rect.center().y - 10.0),
            pos2(right, rect.center().y + 10.0),
        );
        if pill.left() < tile.right() + 80.0 {
            break;
        }
        painter.rect_filled(pill, 10.0, color.gamma_multiply(0.14));
        painter.galley(
            pos2(pill.left() + 8.0, pill.center().y - galley.size().y / 2.0),
            galley,
            *color,
        );
        right = pill.left() - 6.0;
    }
    let text = Rect::from_min_max(
        pos2(tile.right() + 10.0, rect.top()),
        pos2(right - 4.0, rect.bottom()),
    );
    let color = if enabled { TEXT() } else { MUTED() };
    let mut job = text::LayoutJob::simple_singleline(row.title.into(), semibold(13.0), color);
    job.wrap = text::TextWrapping::truncate_at_width(text.width().max(0.0));
    let title = painter.layout_job(job);
    let has_detail = !row.detail.is_empty();
    let title_y = if has_detail {
        rect.center().y - 9.0
    } else {
        rect.center().y
    };
    painter.galley(
        pos2(text.left(), title_y - title.size().y / 2.0),
        title,
        color,
    );
    if has_detail {
        let mut job = text::LayoutJob::simple_singleline(
            row.detail.into(),
            FontId::proportional(11.5),
            FAINT(),
        );
        job.wrap = text::TextWrapping::truncate_at_width(text.width().max(0.0));
        let detail = painter.layout_job(job);
        painter.galley(
            pos2(text.left(), rect.center().y + 9.0 - detail.size().y / 2.0),
            detail,
            FAINT(),
        );
    }
    response.widget_info(|| WidgetInfo::selected(WidgetType::Button, enabled, selected, row.title));
    response
}

/// A tinted note: amber for warnings, red for errors, accent for results.
pub(crate) fn callout(ui: &mut Ui, color: Color32, text: &str) {
    Frame::new()
        .fill(color.gamma_multiply(0.08))
        .stroke(Stroke::new(1.0, color.gamma_multiply(0.3)))
        .corner_radius(8)
        .inner_margin(Margin::symmetric(12, 9))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.add(Label::new(RichText::new(text).size(12.5).color(color)).wrap());
        });
}

/// A refusal in plain words, with its first line as the title. Error codes stay in the
/// text for Activity and automation, and are left out here.
pub(crate) fn refusal(ui: &mut Ui, color: Color32, text: &str) {
    let plain = plain_error(text);
    let (title, rest) = plain.split_once('\n').unwrap_or((plain.as_str(), ""));
    Frame::new()
        .fill(color.gamma_multiply(0.08))
        .stroke(Stroke::new(1.0, color.gamma_multiply(0.3)))
        .corner_radius(8)
        .inner_margin(Margin::symmetric(12, 9))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 2.0;
            if rest.is_empty() {
                ui.add(Label::new(RichText::new(title).size(12.5).color(color)).wrap());
            } else {
                ui.add(Label::new(RichText::new(title).font(semibold(12.5)).color(color)).wrap());
                ui.add(Label::new(RichText::new(rest).size(12.5).color(TEXT_SOFT())).wrap());
            }
        });
}

/// `text` without the codes in front of commit series and rebase refusals.
pub(crate) fn plain_error(text: &str) -> String {
    let plain = |line: &'_ str| -> String {
        let bare = line.strip_prefix("invalid input: ").unwrap_or(line);
        if let Some((code, rest)) = bare.split_once(": ")
            && (code.starts_with("patch_series_") || code.starts_with("series_"))
        {
            return rest.to_owned();
        }
        for prefix in ["Rebase [", "Commit operation ["] {
            if let Some((code, rest)) = line.strip_prefix(prefix).and_then(|l| l.split_once("]: "))
                && !code.contains(' ')
            {
                return rest.to_owned();
            }
        }
        // An Activity detail.
        if let Some((code, rest)) = line.strip_prefix('[').and_then(|l| l.split_once("] "))
            && !code.is_empty()
            && code.chars().all(|c| c.is_ascii_lowercase() || c == '_')
        {
            return rest.to_owned();
        }
        line.to_owned()
    };
    text.lines().map(plain).collect::<Vec<_>>().join("\n")
}

#[cfg(test)]
mod refusal_tests {
    use super::plain_error;

    #[test]
    fn codes_stay_out_of_what_people_read() {
        assert_eq!(plain_error("invalid input: series_dirty: Commit first\nNothing moved."), "Commit first\nNothing moved.");
        assert_eq!(plain_error("invalid input: patch_series_empty: No commits"), "No commits");
        assert_eq!(plain_error("Commit series was not exported.\ninvalid input: patch_series_limit: Too many"), "Commit series was not exported.\nToo many");
        assert_eq!(plain_error("Rebase [conflicts_remain]: Fix the conflicts."), "Fix the conflicts.");
        assert_eq!(plain_error("invalid input: path: something else"), "invalid input: path: something else");
        assert_eq!(plain_error("Rebase [not a code]: kept"), "Rebase [not a code]: kept");
        assert_eq!(plain_error("Commit operation [dirty_worktree]: Commit first."), "Commit first.");
        assert_eq!(plain_error("Revert stopped.\n[conflicts_remain] Fix them."), "Revert stopped.\nFix them.");
        assert_eq!(plain_error("[Not a code] kept"), "[Not a code] kept");
    }
}

/// Where a step of a paused series or rebase stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StepMark {
    Done,
    Stopped,
    LeftOut,
    Waiting,
}

/// One row of `steps`.
pub(crate) struct Step {
    pub mark: StepMark,
    pub number: usize,
    pub subject: String,
    pub note: &'static str,
}

/// A paused series or rebase, a row per commit, scrolled to where it stopped.
pub(crate) fn steps(ui: &mut Ui, id: &str, rows: &[Step]) {
    const HEIGHT: f32 = 32.0;
    Frame::new()
        .stroke(Stroke::new(1.0, CARD_STROKE()))
        .corner_radius(8)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ScrollArea::vertical()
                .id_salt((id, "steps"))
                .max_height(HEIGHT * 6.0)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.spacing_mut().item_spacing.y = 0.0;
                    // Once for each stop: a row past the sixth starts out of view.
                    let seen = ui.id().with("seen");
                    let stop = rows.iter().find(|s| matches!(s.mark, StepMark::Stopped | StepMark::LeftOut)).map(|s| s.number);
                    let first = ui.data(|d| d.get_temp::<Option<usize>>(seen)).flatten() != stop;
                    for (i, step) in rows.iter().enumerate() {
                        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), HEIGHT), Sense::hover());
                        let painter = ui.painter();
                        let waiting = step.mark == StepMark::Waiting;
                        if step.mark == StepMark::Stopped {
                            painter.rect_filled(rect, 0.0, AMBER().gamma_multiply(0.06));
                        }
                        if first && Some(step.number) == stop {
                            ui.scroll_to_rect(rect, Some(Align::Center));
                        }
                        let dot = pos2(rect.left() + 18.0, rect.center().y);
                        match step.mark {
                            StepMark::Done => painter.circle_filled(dot, 5.0, GREEN()),
                            StepMark::Stopped => painter.circle_filled(dot, 5.0, AMBER()),
                            StepMark::LeftOut => painter.circle_stroke(dot, 4.0, Stroke::new(2.0, RED())),
                            StepMark::Waiting => painter.circle_stroke(dot, 4.0, Stroke::new(1.5, RING_IDLE())),
                        };
                        painter.text(
                            pos2(rect.left() + 48.0, rect.center().y),
                            Align2::RIGHT_CENTER,
                            step.number.to_string(),
                            FontId::proportional(12.0),
                            HINT(),
                        );
                        let left = rect.left() + 58.0;
                        let note = painter.layout_no_wrap(step.note.to_owned(), FontId::proportional(12.0), HINT());
                        let note_width = note.size().x;
                        painter.galley(
                            pos2(rect.right() - 12.0 - note_width, rect.center().y - note.size().y / 2.0),
                            note,
                            HINT(),
                        );
                        let ink = if waiting || step.mark == StepMark::LeftOut { MUTED() } else { TEXT() };
                        let mut job = text::LayoutJob::simple_singleline(step.subject.clone(), FontId::proportional(13.0), ink);
                        if step.mark == StepMark::LeftOut {
                            job.sections[0].format.strikethrough = Stroke::new(1.0, MUTED());
                        }
                        job.wrap = text::TextWrapping::truncate_at_width((rect.right() - 24.0 - note_width - left).max(40.0));
                        let subject = painter.layout_job(job);
                        painter.galley(pos2(left, rect.center().y - subject.size().y / 2.0), subject, ink);
                        if i + 1 < rows.len() {
                            painter.hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0, ROW_LINE()));
                        }
                    }
                    ui.data_mut(|d| d.insert_temp(seen, stop));
                });
        });
}

/// A commit a dialog names: dot, subject, then its ID and a note.
pub(crate) fn commit_card(ui: &mut Ui, subject: &str, id: &str, note: &str) {
    Frame::new()
        .fill(WELL())
        .stroke(Stroke::new(1.0, CARD_STROKE()))
        .corner_radius(8)
        .inner_margin(Margin::symmetric(14, 11))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 12.0;
                let (dot, _) = ui.allocate_exact_size(vec2(10.0, 10.0), Sense::hover());
                ui.painter().circle_filled(dot.center(), 5.0, BLUE());
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 1.0;
                    ui.add(Label::new(RichText::new(subject).font(semibold(13.0))).truncate());
                    let mut job = text::LayoutJob::default();
                    job.append(id, 0.0, TextFormat::simple(FontId::monospace(12.0), HINT()));
                    job.append(&format!(" · {note}"), 0.0, TextFormat::simple(FontId::proportional(12.0), HINT()));
                    ui.add(Label::new(job).truncate());
                });
            });
        });
}

/// Small caption above a group of details.
pub(crate) fn caption(ui: &mut Ui, text: &str) {
    ui.label(
        RichText::new(text.to_uppercase())
            .font(semibold(11.0))
            .color(FAINT()),
    );
}

/// One labelled value; values wrap and may be monospace.
pub(crate) fn field(ui: &mut Ui, label: &str, value: &str, mono: bool) {
    let font = if mono {
        FontId::monospace(12.5)
    } else {
        FontId::proportional(12.5)
    };
    let width = ui
        .painter()
        .layout_no_wrap(value.into(), font, TEXT())
        .size()
        .x;
    let text = RichText::new(value).size(12.5);
    let value = Label::new(if mono { text.monospace() } else { text }).wrap();
    // Long IDs and paths get their own line instead of wrapping beside the label.
    if width > ui.available_width() - 112.0 {
        ui.label(RichText::new(label).size(12.0).color(FAINT()));
        ui.add(value);
        return;
    }
    ui.horizontal_top(|ui| {
        let (rect, _) = ui.allocate_exact_size(vec2(104.0, 18.0), Sense::hover());
        ui.painter().text(
            rect.left_center(),
            Align2::LEFT_CENTER,
            label,
            FontId::proportional(12.5),
            FAINT(),
        );
        ui.add(value);
    });
}

/// Explanatory text in the page or review body.
pub(crate) fn note(ui: &mut Ui, text: &str) {
    ui.add(Label::new(RichText::new(text).size(12.0).color(FAINT())).wrap());
}

/// A sunken group in a review body: labelled fields ("what will happen") or a
/// bounded code or file preview.
pub(crate) fn well<R>(ui: &mut Ui, content: impl FnOnce(&mut Ui) -> R) -> R {
    Frame::new()
        .fill(BG())
        .stroke(Stroke::new(1.0, HAIRLINE()))
        .corner_radius(8)
        .inner_margin(Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 6.0;
            content(ui)
        })
        .inner
}

/// A card with a header strip: what a review found, or what will run, then its rows.
pub(crate) fn titled_card(ui: &mut Ui, header: impl FnOnce(&mut Ui), body: impl FnOnce(&mut Ui)) {
    Frame::new()
        .stroke(Stroke::new(1.0, CARD_STROKE()))
        .corner_radius(8)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            Frame::new()
                .fill(CARD())
                .corner_radius(CornerRadius { nw: 7, ne: 7, sw: 0, se: 0 })
                .inner_margin(Margin::symmetric(12, 8))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        header(ui)
                    });
                });
            let top = ui.cursor().top();
            ui.painter()
                .hline(ui.max_rect().x_range(), top, Stroke::new(1.0, CARD_STROKE()));
            Frame::new()
                .inner_margin(Margin::symmetric(12, 10))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.spacing_mut().item_spacing.y = 6.0;
                    body(ui)
                });
        });
}

/// One row of a card: the label in a fixed column and the value beside it, never under it.
pub(crate) fn key_value(ui: &mut Ui, key: &str, width: f32, value: impl FnOnce(&mut Ui)) {
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 12.0;
        let (rect, _) = ui.allocate_exact_size(vec2(width, 18.0), Sense::hover());
        let mut job =
            text::LayoutJob::simple_singleline(key.to_owned(), FontId::proportional(13.0), HINT());
        job.wrap = text::TextWrapping::truncate_at_width(width);
        let galley = ui.painter().layout_job(job);
        ui.painter()
            .galley(pos2(rect.left(), rect.center().y - galley.size().y / 2.0), galley, HINT());
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            value(ui)
        });
    });
}

/// One row of choices drawn as small cards: a radio mark, the name and a line about it.
/// Each card is `(selected, name, about)`; the cards share the row's width and the
/// tallest card's height.
pub(crate) fn option_cards(
    ui: &mut Ui,
    enabled: bool,
    cards: &[(bool, &str, &str)],
) -> Vec<Response> {
    const GAP: f32 = 8.0;
    let count = cards.len().max(1) as f32;
    let width = ((ui.available_width() - GAP * (count - 1.0)) / count).floor();
    let text_width = (width - 44.0).max(40.0);
    let texts: Vec<_> = cards
        .iter()
        .map(|&(_, name, about)| {
            (
                ui.painter()
                    .layout(name.to_owned(), semibold(13.0), TEXT(), text_width),
                ui.painter()
                    .layout(about.to_owned(), FontId::proportional(12.0), HINT(), text_width),
            )
        })
        .collect();
    let height = texts
        .iter()
        .map(|(name, about)| 10.0 + name.size().y + 3.0 + about.size().y + 10.0)
        .fold(58.0, f32::max);
    ui.add_enabled_ui(enabled, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = GAP;
            cards
                .iter()
                .zip(texts)
                .map(|(&(selected, title, _), (name, about))| {
                    let (rect, response) =
                        ui.allocate_exact_size(vec2(width, height), Sense::click());
                    let line = if selected {
                        BLUE()
                    } else if response.hovered() {
                        EDGE_HOT()
                    } else {
                        CARD_STROKE()
                    };
                    let fill = if selected { BLUE().gamma_multiply(0.08) } else { PANEL() };
                    ui.painter()
                        .rect(rect, 8.0, fill, Stroke::new(1.0, line), StrokeKind::Inside);
                    let mark = pos2(rect.left() + 18.0, rect.top() + 10.0 + name.size().y / 2.0);
                    let ring = if selected { BLUE() } else { RING_IDLE() };
                    ui.painter().circle_stroke(mark, 6.5, Stroke::new(1.5, ring));
                    if selected {
                        ui.painter().circle_filled(mark, 3.0, BLUE());
                    }
                    let name_height = name.size().y;
                    ui.painter()
                        .galley(pos2(rect.left() + 32.0, rect.top() + 10.0), name, TEXT());
                    ui.painter().galley(
                        pos2(rect.left() + 32.0, rect.top() + 13.0 + name_height),
                        about,
                        HINT(),
                    );
                    response.widget_info(|| {
                        WidgetInfo::selected(WidgetType::RadioButton, enabled, selected, title)
                    });
                    response
                })
                .collect()
        })
        .inner
    })
    .inner
}

/// Shows or hides more choices; the chevron points down while they show.
pub(crate) fn disclosure(ui: &mut Ui, open: bool, label: &str) -> Response {
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_owned(), FontId::proportional(12.5), HINT());
    let (rect, response) =
        ui.allocate_exact_size(vec2(galley.size().x + 30.0, 26.0), Sense::click());
    if response.hovered() || response.has_focus() {
        ui.painter().rect_filled(rect, 6.0, HOVER());
    }
    let ink = if response.hovered() { TEXT() } else { HINT() };
    icon(
        ui.painter(),
        Rect::from_center_size(pos2(rect.left() + 11.0, rect.center().y), vec2(10.0, 10.0)),
        if open { Icon::Chevron } else { Icon::ChevronRight },
        ink,
    );
    ui.painter().galley_with_override_text_color(
        pos2(rect.left() + 22.0, rect.center().y - galley.size().y / 2.0),
        galley,
        ink,
    );
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, label));
    response
}

// Button labels show without a trailing "…", like menu commands (see `command_label`).
pub(crate) fn primary(ui: &mut Ui, enabled: bool, label: &str) -> Response {
    ui.add_enabled(
        enabled,
        Button::new(
            RichText::new(command_label(label))
                .color(ACCENT_INK())
                .strong(),
        )
        .fill(BLUE()),
    )
}

pub(crate) fn danger(ui: &mut Ui, enabled: bool, label: &str) -> Response {
    ui.add_enabled(
        enabled,
        Button::new(
            RichText::new(command_label(label))
                .color(RED_INK())
                .strong(),
        )
        .fill(RED()),
    )
}

pub(crate) fn danger_ghost(ui: &mut Ui, enabled: bool, label: &str) -> Response {
    ui.add_enabled(
        enabled,
        Button::new(RichText::new(command_label(label)).color(RED()))
            .fill(Color32::TRANSPARENT)
            .stroke(Stroke::new(1.0, RED().gamma_multiply(0.45))),
    )
}

pub(crate) fn ghost(ui: &mut Ui, enabled: bool, label: &str) -> Response {
    ui.add_enabled(enabled, quiet(Button::new(command_label(label))))
}

/// A quiet action with an icon: framed only while hovered or focused, and the same
/// size in every state so neighbouring buttons never move.
pub(crate) fn tool(ui: &mut Ui, image: Icon, label: &str, enabled: bool) -> Response {
    let label = command_label(label);
    let font = FontId::proportional(12.5);
    let width = ui
        .painter()
        .layout_no_wrap(label.into(), font.clone(), TEXT())
        .size()
        .x;
    let response = ui.add_enabled(
        enabled,
        Button::new("")
            .frame(false)
            .min_size(vec2(width + 38.0, 28.0)),
    );
    let rect = response.rect;
    let hot = enabled && (response.hovered() || response.has_focus());
    if hot {
        ui.painter().rect(
            rect,
            6.0,
            HOVER(),
            Stroke::new(1.0, EDGE_HOT()),
            StrokeKind::Inside,
        );
    }
    let ink = if hot { TEXT() } else { MUTED() };
    icon(
        ui.painter(),
        Rect::from_center_size(pos2(rect.left() + 15.5, rect.center().y), vec2(13.0, 13.0)),
        image,
        ink,
    );
    ui.painter().text(
        pos2(rect.left() + 28.0, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        font,
        ink,
    );
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, enabled, label));
    response
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Part {
    Body,
    Footer,
}

pub(crate) struct ReviewHeader<'a> {
    pub image: Icon,
    pub title: &'a str,
    pub subtitle: &'a str,
    pub danger: bool,
}

/// What a review dialog's caller acts on.
pub(crate) struct ReviewResponse {
    modal: ModalResponse<()>,
    /// The header's close button was clicked.
    closed: bool,
}

impl ReviewResponse {
    /// Esc, a click outside, or the header's close button. Callers decide whether
    /// the dialog may close right now, as they always did for Esc.
    pub(crate) fn should_close(&self) -> bool {
        self.closed || self.modal.should_close()
    }

    /// Only the header's close button, for dialogs that ignore clicks outside because
    /// their own resizing can leave the pointer there.
    pub(crate) fn close_clicked(&self) -> bool {
        self.closed
    }
}

/// The shared review dialog: what will happen, then the decision.
/// One closure renders the body and the footer (buttons are laid out right to left).
pub(crate) fn review(
    ctx: &Context,
    id: &str,
    head: ReviewHeader,
    widgets: &mut HashMap<String, UiControl>,
    scroll_key: &str,
    scroll_enabled: bool,
    content: impl FnMut(&mut Ui, Part, &mut HashMap<String, UiControl>),
) -> ReviewResponse {
    review_dialog(
        ctx,
        id,
        640.0,
        head,
        widgets,
        scroll_key,
        scroll_enabled,
        true,
        content,
    )
}

/// [`review`] at up to `width` points, for a dialog whose width follows its contents.
#[allow(clippy::too_many_arguments)]
pub(crate) fn review_at(
    ctx: &Context,
    id: &str,
    width: f32,
    head: ReviewHeader,
    widgets: &mut HashMap<String, UiControl>,
    scroll_key: &str,
    scroll_enabled: bool,
    content: impl FnMut(&mut Ui, Part, &mut HashMap<String, UiControl>),
) -> ReviewResponse {
    review_dialog(ctx, id, width, head, widgets, scroll_key, scroll_enabled, true, content)
}

/// [`review`] at up to 980 points, for a file list beside its changes.
pub(crate) fn split_review(
    ctx: &Context,
    id: &str,
    head: ReviewHeader,
    widgets: &mut HashMap<String, UiControl>,
    scroll_key: &str,
    scroll_enabled: bool,
    content: impl FnMut(&mut Ui, Part, &mut HashMap<String, UiControl>),
) -> ReviewResponse {
    review_dialog(ctx, id, 980.0, head, widgets, scroll_key, scroll_enabled, true, content)
}

/// A review dialog with nothing to decide (About, the shortcut list, release notes, a
/// finished tool): no footer, so the header's × and Esc close it. `Part::Footer` is
/// never drawn.
pub(crate) fn info(
    ctx: &Context,
    id: &str,
    head: ReviewHeader,
    widgets: &mut HashMap<String, UiControl>,
    scroll_key: &str,
    content: impl FnMut(&mut Ui, Part, &mut HashMap<String, UiControl>),
) -> ReviewResponse {
    review_dialog(ctx, id, 640.0, head, widgets, scroll_key, true, false, content)
}

/// [`info`] at up to 860 points, for the release notes.
pub(crate) fn wide_info(
    ctx: &Context,
    id: &str,
    head: ReviewHeader,
    widgets: &mut HashMap<String, UiControl>,
    scroll_key: &str,
    content: impl FnMut(&mut Ui, Part, &mut HashMap<String, UiControl>),
) -> ReviewResponse {
    review_dialog(ctx, id, 860.0, head, widgets, scroll_key, true, false, content)
}

/// [`info`] at up to 1120 points, for Legal: license texts keep their 80-column lines
/// beside the list.
pub(crate) fn full_info(
    ctx: &Context,
    id: &str,
    head: ReviewHeader,
    widgets: &mut HashMap<String, UiControl>,
    scroll_key: &str,
    content: impl FnMut(&mut Ui, Part, &mut HashMap<String, UiControl>),
) -> ReviewResponse {
    review_dialog(ctx, id, 1120.0, head, widgets, scroll_key, true, false, content)
}

/// A dialog without a header, for About: only × in its corner, then the content.
pub(crate) fn plain(
    ctx: &Context,
    id: &str,
    max_width: f32,
    widgets: &mut HashMap<String, UiControl>,
    content: impl FnOnce(&mut Ui, &mut HashMap<String, UiControl>),
) -> ReviewResponse {
    let width = (ctx.content_rect().width() - 48.0).clamp(300.0, max_width);
    let mut closed = false;
    let modal = Modal::new(Id::new(id))
        .backdrop_color(Color32::from_black_alpha(120))
        .frame(modal_frame())
        .show(ctx, |ui| {
            ui.set_width(width);
            Frame::new()
                .inner_margin(Margin {
                    left: 22,
                    right: 14,
                    top: 14,
                    bottom: 22,
                })
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.allocate_ui_with_layout(
                        vec2(ui.available_width(), 28.0),
                        Layout::right_to_left(Align::Center),
                        |ui| {
                            let close = crate::chrome::icon_button(ui, Icon::Close, true, false, 28.0)
                                .on_hover_text("Close (Esc)");
                            control(widgets, "dialog-close", &close, ui.clip_rect());
                            control(widgets, "dialog-cancel", &close, ui.clip_rect());
                            closed = close.clicked();
                        },
                    );
                    Frame::new()
                        .inner_margin(Margin {
                            left: 0,
                            right: 8,
                            top: 0,
                            bottom: 0,
                        })
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.spacing_mut().item_spacing.y = 7.0;
                            content(ui, widgets);
                        });
                });
        });
    ReviewResponse { modal, closed }
}

/// The review dialog at up to 860 points, for side-by-side file previews, the
/// reflog and the interactive rebase plan.
pub(crate) fn wide_review(
    ctx: &Context,
    id: &str,
    head: ReviewHeader,
    widgets: &mut HashMap<String, UiControl>,
    scroll_key: &str,
    scroll_enabled: bool,
    content: impl FnMut(&mut Ui, Part, &mut HashMap<String, UiControl>),
) -> ReviewResponse {
    review_dialog(
        ctx,
        id,
        860.0,
        head,
        widgets,
        scroll_key,
        scroll_enabled,
        true,
        content,
    )
}

/// The review dialog at up to 1120 points, for a list beside its details (the reflog).
pub(crate) fn full_review(
    ctx: &Context,
    id: &str,
    head: ReviewHeader,
    widgets: &mut HashMap<String, UiControl>,
    scroll_key: &str,
    scroll_enabled: bool,
    content: impl FnMut(&mut Ui, Part, &mut HashMap<String, UiControl>),
) -> ReviewResponse {
    review_dialog(
        ctx,
        id,
        1120.0,
        head,
        widgets,
        scroll_key,
        scroll_enabled,
        true,
        content,
    )
}

/// Brings a dialog opened from another dialog above it. egui raises an area as it reappears,
/// but a press on the dialog below in that same frame ties with the raise, the older order wins
/// and the new dialog stays hidden behind. Call this for the new dialog's first two frames.
pub(crate) fn raise(ctx: &Context, id: &str) {
    ctx.move_to_top(LayerId::new(Order::Foreground, Id::new(id)));
    ctx.request_repaint();
}

/// The raised surface of review dialogs and the guide.
pub(crate) fn modal_frame() -> Frame {
    Frame::new()
        .fill(WINDOW())
        .stroke(Stroke::new(1.0, WINDOW_EDGE()))
        .corner_radius(12)
        .shadow(epaint::Shadow {
            offset: [0, 18],
            blur: 44,
            spread: 0,
            color: Color32::from_black_alpha(140),
        })
}

#[allow(clippy::too_many_arguments)]
fn review_dialog(
    ctx: &Context,
    id: &str,
    max_width: f32,
    head: ReviewHeader,
    widgets: &mut HashMap<String, UiControl>,
    scroll_key: &str,
    scroll_enabled: bool,
    footer: bool,
    mut content: impl FnMut(&mut Ui, Part, &mut HashMap<String, UiControl>),
) -> ReviewResponse {
    let width = (ctx.content_rect().width() - 48.0).clamp(300.0, max_width);
    let height = (ctx.content_rect().height() - 250.0).max(100.0);
    let mut closed = false;
    let modal = Modal::new(Id::new(id))
        .backdrop_color(Color32::from_black_alpha(120))
        .frame(modal_frame())
        .show(ctx, |ui| {
            ui.set_width(width);
            ui.spacing_mut().item_spacing.y = 0.0;
            let tint = if head.danger { RED() } else { BLUE() };
            // Background explanations live in the guide; "?" opens its topic.
            let help = crate::guide::available(ctx);
            Frame::new()
                .inner_margin(Margin {
                    left: 22,
                    right: 22,
                    top: 20,
                    bottom: 6,
                })
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal_top(|ui| {
                        let (badge, _) = ui.allocate_exact_size(vec2(38.0, 38.0), Sense::hover());
                        ui.painter()
                            .circle_filled(badge.center(), 19.0, tint.gamma_multiply(0.15));
                        icon(ui.painter(), badge.shrink(10.0), head.image, tint);
                        ui.add_space(8.0);
                        let text_width = ui.available_width() - if help { 70.0 } else { 36.0 };
                        ui.vertical(|ui| {
                            ui.set_max_width(text_width);
                            ui.spacing_mut().item_spacing.y = 3.0;
                            ui.add(
                                Label::new(RichText::new(head.title).font(semibold(16.0))).wrap(),
                            );
                            if !head.subtitle.is_empty() {
                                ui.add(
                                    Label::new(RichText::new(head.subtitle).color(MUTED())).wrap(),
                                );
                            }
                        });
                        ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                            let close = crate::chrome::icon_button(ui, Icon::Close, true, false, 28.0)
                                .on_hover_text("Close (Esc)");
                            control(widgets, "dialog-close", &close, ui.clip_rect());
                            // Automation dismissed dialogs with their footer Cancel or
                            // Close. A footer Stop or Back drawn later keeps the name.
                            control(widgets, "dialog-cancel", &close, ui.clip_rect());
                            closed = close.clicked();
                            if help {
                                crate::guide::header_button(ui);
                            }
                        });
                    });
                });
            let body = ScrollArea::vertical()
                .id_salt((id, "body"))
                .max_height(height)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    Frame::new()
                        .inner_margin(Margin {
                            left: 22,
                            right: 22,
                            top: 10,
                            bottom: 4,
                        })
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.spacing_mut().item_spacing.y = 7.0;
                            content(ui, Part::Body, widgets);
                        });
                });
            region(
                widgets,
                scroll_key,
                body.inner_rect.intersect(ui.clip_rect()),
                scroll_enabled,
            );
            if !footer {
                ui.add_space(18.0);
                return;
            }
            ui.add_space(14.0);
            let footer = Frame::new()
                .fill(SUNKEN())
                .inner_margin(Margin::symmetric(22, 12))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    // One control row high. The modal is sized from last frame's
                    // content, so a centered layout filling the available height would
                    // keep the dialog from ever shrinking (e.g. after an error clears).
                    ui.allocate_ui_with_layout(
                        vec2(ui.available_width(), ui.spacing().interact_size.y),
                        Layout::right_to_left(Align::Center),
                        |ui| {
                            ui.spacing_mut().item_spacing.x = 8.0;
                            content(ui, Part::Footer, widgets);
                        },
                    );
                })
                .response
                .rect;
            ui.painter()
                .hline(footer.x_range(), footer.top(), Stroke::new(1.0, HAIRLINE()));
        });
    ReviewResponse { modal, closed }
}

/// Work under way. Alone in its panel (nothing drawn above it) it is the panel's loading
/// state, a spinner over its text centered in a block; under other content it stays a
/// status line.
pub(crate) fn waiting(ui: &mut Ui, text: &str) {
    // The cursor tells: panels grow their min_rect to the whole panel before content.
    if ui.layout().main_dir() == Direction::TopDown && ui.cursor().top() - ui.max_rect().top() < 1.0 {
        centered(ui, text, 120.0, 0.0, |_| ());
    } else {
        busy_line(ui, text);
    }
}

/// A spinner and its text on one line, beside content that is still shown.
pub(crate) fn busy_line(ui: &mut Ui, text: &str) {
    ui.horizontal(|ui| {
        ui.spinner();
        ui.label(RichText::new(text).color(MUTED()));
    });
}

/// A pane or page with nothing to show yet: a spinner over its text, centered in the rest
/// of the pane.
pub(crate) fn loading(ui: &mut Ui, text: &str) {
    loading_with(ui, text, 0.0, |_| ());
}

/// [`loading`] with `below` under the text, such as Stop, `extra` points tall.
pub(crate) fn loading_with(ui: &mut Ui, text: &str, extra: f32, below: impl FnOnce(&mut Ui)) {
    // An unbounded scroll area has no "rest of the pane"; a block stands in.
    let height = ui.available_height();
    let height = if height.is_finite() && height < 4000.0 { height.max(120.0) } else { 160.0 };
    centered(ui, text, height, extra, below);
}

fn centered(ui: &mut Ui, text: &str, height: f32, extra: f32, below: impl FnOnce(&mut Ui)) {
    let width = ui.available_width();
    let galley = ui.painter().layout(text.to_owned(), FontId::proportional(14.0), TEXT(), (width - 32.0).max(40.0));
    // The spinner, a gap, the text and what goes below, centered as one group.
    let group = 26.0 + 10.0 + galley.size().y + extra;
    ui.allocate_ui_with_layout(vec2(width, height), Layout::top_down(Align::Center), |ui| {
        ui.set_min_size(vec2(width, height));
        ui.add_space(((height - group) / 2.0).max(8.0));
        ui.add(Spinner::new().size(26.0));
        ui.add_space(10.0 - ui.spacing().item_spacing.y);
        ui.label(RichText::new(text).size(14.0).color(TEXT()));
        below(ui);
    });
}

/// A review footer's status line, added after its buttons so it takes the width
/// left of them. `busy` adds the spinner.
pub(crate) fn footer_status(ui: &mut Ui, busy: bool, text: &str) {
    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        if busy {
            ui.add(Spinner::new().size(14.0));
        }
        ui.add(Label::new(RichText::new(text).size(12.5).color(HINT())).truncate());
    });
}

/// A one-line label cut to `width` and kept at the left of that space, so what
/// follows it in the row lines up on the right.
pub(crate) fn left_label(ui: &mut Ui, width: f32, label: Label) -> Response {
    ui.allocate_ui_with_layout(vec2(width, 18.0), Layout::left_to_right(Align::Center), |ui| {
        ui.set_min_width(width);
        ui.add(label.truncate())
    })
    .inner
}

/// Running text with a Learn more link to the Help guide right after its last
/// word: the link has the text's size and line, with no button padding, and moves
/// to the next line whole when it doesn't fit. Without a guide topic only the text
/// shows. Returns the text and, when shown, the link.
pub(crate) fn with_guide_link(
    ui: &mut Ui,
    texts: &[&str],
    size: f32,
    color: Color32,
    widgets: &mut HashMap<String, UiControl>,
    id: &str,
) -> (Response, Option<Response>) {
    let show = crate::guide::available(ui.ctx());
    paragraph_with_link(ui, texts, size, color, show, widgets, id)
}

/// [`with_guide_link`]'s layout, with the link shown or not.
fn paragraph_with_link(
    ui: &mut Ui,
    texts: &[&str],
    size: f32,
    color: Color32,
    show: bool,
    widgets: &mut HashMap<String, UiControl>,
    id: &str,
) -> (Response, Option<Response>) {
    ui.horizontal_wrapped(|ui| {
        // One label for all the sentences, ending in a space before the link, so they
        // wrap as one paragraph.
        ui.spacing_mut().item_spacing.x = 0.0;
        let text = ui.label(RichText::new(format!("{} ", texts.join(" "))).size(size).color(color));
        let link = show.then(|| {
            let r = ui
                .add(
                    Label::new(RichText::new("Learn more").size(size).color(BLUE()))
                        .sense(Sense::click())
                        .wrap_mode(TextWrapMode::Extend),
                )
                .on_hover_cursor(CursorIcon::PointingHand);
            if r.hovered() || r.has_focus() {
                ui.painter().hline(r.rect.x_range(), r.rect.bottom() - 1.0, Stroke::new(1.0, BLUE()));
            }
            control(widgets, id, &r, ui.clip_rect());
            if r.clicked() {
                crate::guide::request(ui.ctx());
            }
            r
        });
        (text, link)
    })
    .inner
}

/// The running operation as its dialog's footer shows it, read before the dialog
/// borrows its state.
pub(crate) struct Running {
    status: String,
    can_stop: bool,
    stop: &'static str,
}

impl Running {
    pub(crate) fn of(write: Option<&ActiveWrite>) -> Self {
        match write {
            Some(w) if w.progress.is_empty() => Self {
                status: w.title.clone(),
                can_stop: w.can_cancel(),
                stop: w.cancel_label(),
            },
            Some(w) => Self {
                status: format!("{} · {}", w.title, w.progress),
                can_stop: w.can_cancel(),
                stop: w.cancel_label(),
            },
            None => Self {
                status: "Finishing".into(),
                can_stop: false,
                stop: "Stop",
            },
        }
    }
}

/// A dialog footer while its operation runs: only Stop and the status line with the
/// operation's one spinner; the status bar and Activity just name it. Stop is
/// published as `cancel-operation` and as the dialog's own `ids`, which automation
/// already uses. Returns whether Stop was clicked.
pub(crate) fn running_footer(
    ui: &mut Ui,
    widgets: &mut HashMap<String, UiControl>,
    running: &Running,
    ids: &[&str],
) -> bool {
    let r = danger_ghost(ui, running.can_stop, running.stop);
    for id in ids.iter().copied().chain(["cancel-operation"]) {
        control(widgets, id, &r, ui.clip_rect());
    }
    footer_status(ui, true, &running.status);
    r.clicked()
}

/// Selected-for-remote style toggle used in page toolbars.
pub(crate) fn choice(ui: &mut Ui, enabled: bool, selected: bool, label: &str) -> Response {
    ui.add_enabled(enabled, Button::selectable(selected, command_label(label)))
}

/// Shared empty-state line for lists.
pub(crate) fn empty(ui: &mut Ui, text: &str) {
    ui.add_space(12.0);
    ui.label(RichText::new(text).color(FAINT()));
}

/// Pill colors used across pages.
#[allow(non_snake_case)]
pub(crate) fn OK() -> Color32 {
    GREEN()
}
#[allow(non_snake_case)]
pub(crate) fn WARN() -> Color32 {
    AMBER()
}
#[allow(non_snake_case)]
pub(crate) fn INFO() -> Color32 {
    BLUE()
}
#[allow(non_snake_case)]
pub(crate) fn NEUTRAL() -> Color32 {
    MUTED()
}

/// The page [`header`] and its hairline, or one compact title row in short panes
/// (for example with the terminal open). Actions are laid out right to left.
pub(crate) fn page_title(
    ui: &mut Ui,
    short: bool,
    image: Icon,
    title: &str,
    description: &str,
    actions: impl FnOnce(&mut Ui),
) {
    if !short {
        header(ui, image, title, description, actions);
        divider(ui);
        return;
    }
    strip(ui, |ui| {
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            actions(ui);
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                ui.label(RichText::new(title).font(semibold(13.0)));
                ui.add(Label::new(RichText::new(description).size(12.0).color(FAINT())).truncate());
            });
        });
    });
}

/// Full-width hairline, e.g. under a page header.
pub(crate) fn divider(ui: &mut Ui) {
    let (line, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter()
        .hline(line.x_range(), line.center().y, Stroke::new(1.0, HAIRLINE()));
}

/// A 40-point pane title row closed by a hairline; actions sit on the right.
pub(crate) fn bar(ui: &mut Ui, title: &str, detail: &str, actions: impl FnOnce(&mut Ui)) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 40.0), Sense::hover());
    ui.painter().hline(
        rect.x_range(),
        rect.bottom() - 0.5,
        Stroke::new(1.0, HAIRLINE()),
    );
    let inner = rect.shrink2(vec2(10.0, 0.0));
    let right = ui
        .scope_builder(
            UiBuilder::new()
                .max_rect(inner)
                .layout(Layout::right_to_left(Align::Center)),
            |ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                actions(ui);
                ui.min_rect().left()
            },
        )
        .inner;
    let right = if right < inner.right() - 1.0 {
        right - 8.0
    } else {
        inner.right()
    };
    let title_galley = ui
        .painter()
        .layout_no_wrap(title.into(), semibold(13.0), TEXT());
    let title_width = title_galley.size().x;
    ui.painter().with_clip_rect(rect).galley(
        pos2(
            rect.left() + 14.0,
            rect.center().y - title_galley.size().y / 2.0,
        ),
        title_galley,
        TEXT(),
    );
    let detail_left = rect.left() + 22.0 + title_width;
    if !detail.is_empty() && detail_left < right - 20.0 {
        clipped_text(
            ui,
            Rect::from_min_max(pos2(detail_left, rect.top()), pos2(right, rect.bottom())),
            detail,
            12.5,
            FAINT(),
        );
    }
}

/// Viewer header row (path, metadata and actions) on the pane fill, closed by a hairline.
pub(crate) fn strip<R>(ui: &mut Ui, content: impl FnOnce(&mut Ui) -> R) -> R {
    let response = Frame::new()
        .fill(PANEL())
        .inner_margin(Margin::symmetric(12, 6))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                content(ui)
            })
            .inner
        });
    let rect = response.response.rect;
    ui.painter().hline(
        rect.x_range(),
        rect.bottom() - 0.5,
        Stroke::new(1.0, HAIRLINE()),
    );
    response.inner
}

/// A segmented control: options in one raised group, the selected one filled.
/// Returns one response per option, in order; callers register and act on them.
/// Inside a right-to-left layout the options still read left to right.
pub(crate) fn segments(ui: &mut Ui, items: &[(&str, bool)]) -> Vec<Response> {
    let reversed = ui.layout().prefer_right_to_left();
    Frame::new()
        .fill(RAISED())
        .stroke(Stroke::new(1.0, HAIRLINE()))
        .corner_radius(7)
        .inner_margin(2)
        .show(ui, |ui| {
            // 22-point options in a 2-point frame: the group is a control's height
            // (CONTROL), so it lines up with the fields beside it. The row takes its
            // height from interact_size when it starts, so set it first.
            ui.spacing_mut().interact_size.y = 22.0;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                // With 3 points of padding, 12.5-point labels would make them 23.
                ui.spacing_mut().button_padding = vec2(10.0, 2.0);
                let mut add = |(label, selected): &(&str, bool)| {
                    let text = RichText::new(*label).size(12.5).color(if *selected {
                        TEXT()
                    } else {
                        MUTED()
                    });
                    let button = Button::new(text)
                        .selected(*selected)
                        .fill(if *selected {
                            SELECTED()
                        } else {
                            Color32::TRANSPARENT
                        })
                        .corner_radius(5)
                        .min_size(vec2(0.0, 22.0));
                    // Unselected options frame only on hover, at a fixed size.
                    if *selected {
                        ui.add(button)
                    } else {
                        ui.add(quiet(button))
                    }
                };
                if reversed {
                    let mut responses: Vec<_> = items.iter().rev().map(&mut add).collect();
                    responses.reverse();
                    responses
                } else {
                    items.iter().map(add).collect()
                }
            })
            .inner
        })
        .inner
}

/// [`segments`] at the height of a field, each option with an optional dimmer count.
pub(crate) fn tab_segments(ui: &mut Ui, items: &[(&str, Option<usize>, bool)]) -> Vec<Response> {
    let reversed = ui.layout().prefer_right_to_left();
    Frame::new()
        .fill(RAISED())
        .stroke(Stroke::new(1.0, HAIRLINE()))
        .corner_radius(7)
        .inner_margin(2)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                ui.spacing_mut().interact_size.y = CONTROL - 6.0;
                ui.spacing_mut().button_padding = vec2(10.0, 2.0);
                let mut add = |(label, count, selected): &(&str, Option<usize>, bool)| {
                    let font = FontId::proportional(12.5);
                    let ink = if *selected { TEXT() } else { MUTED() };
                    let mut job = text::LayoutJob::default();
                    job.append(label, 0.0, TextFormat::simple(font.clone(), ink));
                    if let Some(count) = count {
                        let tint = if *selected { MUTED() } else { HINT() };
                        job.append(&format!("  {}", number_text(*count)), 0.0, TextFormat::simple(font, tint));
                    }
                    let button = Button::new(job)
                        .selected(*selected)
                        .fill(if *selected { SELECTED() } else { Color32::TRANSPARENT })
                        .corner_radius(5)
                        .min_size(vec2(0.0, CONTROL - 6.0));
                    if *selected {
                        ui.add(button)
                    } else {
                        ui.add(quiet(button))
                    }
                };
                if reversed {
                    let mut responses: Vec<_> = items.iter().rev().map(&mut add).collect();
                    responses.reverse();
                    responses
                } else {
                    items.iter().map(add).collect()
                }
            })
            .inner
        })
        .inner
}

/// [`tab_segments`] where every option has a count.
pub(crate) fn count_segments(ui: &mut Ui, items: &[(&str, usize, bool)]) -> Vec<Response> {
    let items: Vec<_> = items.iter().map(|(label, n, on)| (*label, Some(*n), *on)).collect();
    tab_segments(ui, &items)
}

/// `2469` as "2,469".
pub(crate) fn number_text(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// The frame of a line across a page; `action` adds what sits at its end.
fn strip_frame<R>(
    ui: &mut Ui,
    color: Color32,
    mark: Option<Icon>,
    text: &str,
    action: impl FnOnce(&mut Ui) -> R,
) -> R {
    Frame::new()
        .fill(color.gamma_multiply(0.08))
        .stroke(Stroke::new(1.0, color.gamma_multiply(0.35)))
        .corner_radius(8)
        .inner_margin(Margin {
            left: 12,
            right: 8,
            top: 4,
            bottom: 4,
        })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let out = action(ui);
                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                        if let Some(mark) = mark {
                            let (rect, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
                            icon(ui.painter(), rect, mark, color);
                        }
                        ui.add(Label::new(RichText::new(text).size(12.5).color(color)).wrap());
                    });
                    out
                })
                .inner
            })
            .inner
        })
        .inner
}

/// A line across a page about what just happened, with one action at its end. Returns
/// the action. The text may run to a second line.
pub(crate) fn notice_strip(ui: &mut Ui, color: Color32, text: &str, action: Option<&str>) -> Option<Response> {
    strip_frame(ui, color, None, text, |ui| action.map(|label| ui.add(Button::new(label))))
}

/// A line saying something worked, closed by its ×. Returns whether it was closed.
pub(crate) fn done_strip(
    ui: &mut Ui,
    color: Color32,
    text: &str,
    widgets: &mut HashMap<String, UiControl>,
    id: &str,
) -> bool {
    strip_frame(ui, color, Some(Icon::Check), text, |ui| {
        let r = crate::chrome::icon_button(ui, Icon::Close, true, false, 24.0).on_hover_text("Dismiss");
        control(widgets, id, &r, ui.clip_rect());
        r.clicked()
    })
}

/// The dialog over a test window while MCP's click waits on its checks. `step` is the
/// check running now; earlier ones are done. Returns whether Stop was clicked.
pub fn input_check(ctx: &Context, kind: &str, steps: &[&str], step: usize, stopping: bool) -> bool {
    let mut stop = false;
    Modal::new(Id::new("mcp-input-check"))
        .backdrop_color(Color32::from_black_alpha(120))
        .frame(modal_frame())
        .show(ctx, |ui| {
            ui.set_width(420.0_f32.min(ctx.content_rect().width() - 48.0));
            ui.spacing_mut().item_spacing.y = 0.0;
            Frame::new()
                .inner_margin(Margin { left: 18, right: 18, top: 16, bottom: 8 })
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        let (tile, _) = ui.allocate_exact_size(vec2(40.0, 40.0), Sense::hover());
                        ui.painter().rect_filled(tile, 10.0, RAISED());
                        icon(ui.painter(), tile.shrink(11.0), Icon::Shield, BLUE());
                        ui.add_space(4.0);
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 1.0;
                            ui.label(RichText::new("MCP input check").font(semibold(15.0)));
                            ui.label(RichText::new(format!("{kind} · nothing has started")).color(MUTED()));
                        });
                    });
                    ui.add_space(8.0);
                    for (i, label) in steps.iter().enumerate() {
                        ui.horizontal(|ui| {
                            ui.set_height(28.0);
                            ui.spacing_mut().item_spacing.x = 10.0;
                            let (mark, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
                            let (font, ink) = if i < step {
                                let center = mark.center();
                                let stroke = Stroke::new(1.4, GREEN().gamma_multiply(0.72));
                                ui.painter().circle_stroke(center, 6.5, stroke);
                                ui.painter().line_segment([center + vec2(-3.0, 0.2), center + vec2(-0.8, 2.4)], stroke);
                                ui.painter().line_segment([center + vec2(-0.8, 2.4), center + vec2(3.2, -2.2)], stroke);
                                (FontId::proportional(13.0), TEXT_SOFT())
                            } else if i == step {
                                Spinner::new().size(15.0).paint_at(ui, mark);
                                (semibold(13.0), TEXT())
                            } else {
                                ui.painter().circle_stroke(mark.center(), 6.5, Stroke::new(1.4, BORDER()));
                                (FontId::proportional(13.0), HINT())
                            };
                            ui.label(RichText::new(*label).font(font).color(ink));
                        });
                    }
                });
            let (rule, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
            ui.painter().hline(rule.x_range(), rule.center().y, Stroke::new(1.0, HAIRLINE()));
            Frame::new()
                .inner_margin(Margin::symmetric(18, 10))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        stop = danger_ghost(ui, !stopping, if stopping { "Stopping" } else { "Stop" }).clicked();
                        footer_status(ui, false, if stopping { "Stopping the check" } else { "Esc also stops it" });
                    });
                });
        });
    stop
}

/// A framed button with an icon, or a spinner when there is none, `width` wide. One this
/// narrow shows only the icon.
pub(crate) fn slot_button(ui: &mut Ui, image: Option<Icon>, label: &str, width: f32, enabled: bool) -> Response {
    let response = ui.add_enabled(enabled, Button::new("").min_size(vec2(width, CONTROL)));
    let rect = response.rect;
    let ink = if enabled { TEXT() } else { FAINT() };
    let compact = width < 40.0;
    let text = ui.painter().layout_no_wrap(label.to_owned(), FontId::proportional(13.0), ink);
    let spread = if compact { 14.0 } else { 14.0 + 6.0 + text.size().x };
    let left = rect.center().x - spread / 2.0;
    let mark = Rect::from_center_size(pos2(left + 7.0, rect.center().y), vec2(14.0, 14.0));
    match image {
        Some(image) => icon(ui.painter(), mark, image, ink),
        None => Spinner::new().size(14.0).paint_at(ui, mark),
    }
    if !compact {
        ui.painter().galley(pos2(left + 20.0, rect.center().y - text.size().y / 2.0), text, ink);
    }
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, enabled, label));
    if compact { response.on_hover_text(label) } else { response }
}

/// A search field at the height of a field, its icon inside.
pub(crate) fn search_box(ui: &mut Ui, value: &mut String, id: &str, hint: &str, width: f32) -> Response {
    let r = ui.add_sized(
        [width, CONTROL],
        TextEdit::singleline(value)
            .id_salt(id)
            .hint_text(hint)
            .char_limit(128)
            .margin(Margin {
                left: 26,
                right: 8,
                top: 0,
                bottom: 0,
            })
            .vertical_align(Align::Center),
    );
    icon(
        ui.painter(),
        Rect::from_center_size(pos2(r.rect.left() + 14.0, r.rect.center().y), vec2(13.0, 13.0)),
        Icon::Search,
        FAINT(),
    );
    r
}

/// A card's fill and outline at `rect`, for a card that fills its space.
pub(crate) fn card_back(ui: &Ui, rect: Rect) {
    ui.painter().rect(rect, 8.0, PANEL(), Stroke::new(1.0, CARD_STROKE()), StrokeKind::Inside);
}

/// A card's header bar across the top of `ui`, its content from the left.
pub(crate) fn card_bar(ui: &mut Ui, content: impl FnOnce(&mut Ui)) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 40.0), Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius { nw: 7, ne: 7, sw: 0, se: 0 }, CARD());
    ui.painter().hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0, CARD_LINE()));
    let inner = Rect::from_min_max(rect.min + vec2(14.0, 0.0), rect.max - vec2(8.0, 0.0));
    let mut bar = ui.new_child(UiBuilder::new().max_rect(inner).layout(Layout::left_to_right(Align::Center)));
    bar.spacing_mut().item_spacing.x = 8.0;
    content(&mut bar);
}

/// An inline status pill such as "2 conflicts" or "at a1c9e42".
pub(crate) fn pill(ui: &mut Ui, text: &str, color: Color32) -> Response {
    let galley = ui
        .painter()
        .layout_no_wrap(text.into(), semibold(11.0), color);
    let (rect, response) =
        ui.allocate_exact_size(vec2(galley.size().x + 16.0, 20.0), Sense::hover());
    ui.painter()
        .rect_filled(rect, 10.0, color.gamma_multiply(0.14));
    ui.painter().galley(
        pos2(rect.left() + 8.0, rect.center().y - galley.size().y / 2.0),
        galley,
        color,
    );
    response
}

/// A 30-point list row: small icon, label and dimmed detail, accent bar when selected.
pub(crate) fn list_row(
    ui: &mut Ui,
    image: Icon,
    label: &str,
    detail: &str,
    selected: bool,
) -> Response {
    let response = ui.add(
        Button::new("")
            .frame(false)
            .min_size(vec2(ui.available_width(), 30.0)),
    );
    let rect = response.rect;
    let hovered = response.hovered() || response.has_focus();
    row_background(ui.painter(), rect, selected, hovered);
    if selected {
        ui.painter().rect_filled(
            Rect::from_min_size(rect.min + vec2(0.0, 7.0), vec2(3.0, rect.height() - 14.0)),
            1.5,
            BLUE(),
        );
    }
    icon(
        ui.painter(),
        Rect::from_center_size(pos2(rect.left() + 18.0, rect.center().y), vec2(14.0, 14.0)),
        image,
        if selected { BLUE() } else { MUTED() },
    );
    let text = Rect::from_min_max(
        pos2(rect.left() + 34.0, rect.top()),
        pos2(rect.right() - 8.0, rect.bottom()),
    );
    let mut job =
        text::LayoutJob::simple_singleline(label.into(), FontId::proportional(13.0), TEXT());
    job.wrap = text::TextWrapping::truncate_at_width(text.width().max(0.0));
    let galley = ui.painter().layout_job(job);
    let width = galley.size().x;
    ui.painter().galley(
        pos2(text.left(), rect.center().y - galley.size().y / 2.0),
        galley,
        TEXT(),
    );
    let detail_left = text.left() + width + 8.0;
    if !detail.is_empty() && detail_left < text.right() - 20.0 {
        clipped_text(
            ui,
            Rect::from_min_max(pos2(detail_left, rect.top()), text.right_bottom()),
            detail,
            12.0,
            FAINT(),
        );
    }
    response.widget_info(|| WidgetInfo::selected(WidgetType::Button, true, selected, label));
    response
}

/// Resizable list pane left of a viewer, as in Changes: pane fill and a hairline edge.
pub(crate) fn list_pane(
    ui: &mut Ui,
    id: Id,
    default: f32,
    range: std::ops::RangeInclusive<f32>,
    content: impl FnOnce(&mut Ui),
) {
    let rect = Panel::left(id)
        .default_size(default)
        .size_range(range)
        .resizable(true)
        .frame(Frame::new().fill(PANEL()))
        .show(ui, content)
        .response
        .rect;
    ui.painter().vline(
        rect.right() - 0.5,
        rect.y_range(),
        Stroke::new(1.0, HAIRLINE()),
    );
}

/// Line-number gutter cell for source viewers: right-aligned, dimmed monospace.
pub(crate) fn gutter(ui: &mut Ui, number: &str, width: f32, height: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    ui.painter().text(
        pos2(rect.right() - 10.0, rect.center().y),
        Align2::RIGHT_CENTER,
        number,
        FontId::monospace(12.0),
        FAINT(),
    );
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Status {
    Done,
    Review,
    Failed,
}

/// Round status mark for activity rows: check, exclamation or cross.
pub(crate) fn status_icon(painter: &Painter, center: Pos2, status: Status) {
    let color = match status {
        Status::Done => GREEN(),
        Status::Review => AMBER(),
        Status::Failed => RED(),
    };
    painter.circle_filled(center, 10.0, color.gamma_multiply(0.16));
    let stroke = Stroke::new(1.5, color);
    match status {
        Status::Done => {
            icon(
                painter,
                Rect::from_center_size(center, vec2(10.0, 10.0)),
                Icon::Check,
                color,
            );
        }
        Status::Review => {
            painter.line_segment([center + vec2(0.0, -5.0), center + vec2(0.0, 1.5)], stroke);
            painter.circle_filled(center + vec2(0.0, 4.5), 1.0, color);
        }
        Status::Failed => {
            painter.line_segment([center + vec2(-3.5, -3.5), center + vec2(3.5, 3.5)], stroke);
            painter.line_segment([center + vec2(3.5, -3.5), center + vec2(-3.5, 3.5)], stroke);
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        app::{DialogKind, Intent, Rocket, View},
        test_common as common,
    };
    use eframe::egui::{Context, Event, Key, Modifiers, Pos2, RawInput, Rect, vec2};
    use std::{
        thread,
        time::{Duration, Instant},
    };

    #[test]
    fn browsing_pages_leave_input_free_and_reviews_keep_it() {
        common::isolated(
            "ROCKET_PAGES_UI_CASE",
            "pages::tests::browsing_pages_leave_input_free_and_reviews_keep_it",
            || {
                let fixture = common::Fixture::new();
                fixture.write("file", "first");
                fixture.commit("first");
                fixture.run(&["tag", "v1"]);
                let ctx = Context::default();
                let mut app = Rocket::with_log_directory(
                    &ctx,
                    vec![fixture.repository.clone()],
                    fixture.root.join("logs"),
                );
                let wait = |app: &mut Rocket| {
                    let deadline = Instant::now() + Duration::from_secs(40);
                    while app.busy() {
                        app.poll(&ctx);
                        assert!(Instant::now() < deadline);
                        thread::sleep(Duration::from_millis(10));
                    }
                };
                wait(&mut app);
                app.auto_refresh = false;
                app.dispatch(Intent::Dialog(DialogKind::Tags));
                wait(&mut app);
                assert_eq!(app.page(), Some(super::Page::Tags));
                assert!(
                    !app.blocking_dialog(),
                    "a browsing page must not block input"
                );

                // Ctrl+Enter must never commit from the Changes form hidden behind a page.
                let mut intents = Vec::new();
                let mut input = RawInput::default();
                input.events.push(Event::Key {
                    key: Key::Enter,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: Modifiers::CTRL,
                });
                let mut output = ctx.run_ui(input, |ui| app.shortcuts(ui.ctx(), &mut intents));
                output.textures_delta.clear();
                assert!(intents.is_empty(), "Ctrl+Enter acted behind a page");

                // Reopening the same page keeps its selection and list.
                let entry = app
                    .dialog
                    .as_ref()
                    .unwrap()
                    .tags
                    .as_ref()
                    .unwrap()
                    .page
                    .as_ref()
                    .unwrap()
                    .entries[0]
                    .clone();
                app.dialog.as_mut().unwrap().tags.as_mut().unwrap().selected = Some(entry.clone());
                app.dispatch(Intent::Dialog(DialogKind::Tags));
                assert_eq!(
                    app.dialog
                        .as_ref()
                        .unwrap()
                        .tags
                        .as_ref()
                        .unwrap()
                        .selected
                        .as_ref()
                        .map(|e| &e.name),
                    Some(&entry.name)
                );

                // A review owns input, and navigation does not discard it.
                app.dialog.as_mut().unwrap().tags.as_mut().unwrap().action =
                    Some(rocket_git::TagAction::Delete);
                assert!(app.blocking_dialog());
                app.dispatch(Intent::View(View::Commits));
                assert!(app.dialog.is_some(), "navigation must not drop a review");

                // Leaving the review returns to the page; navigation then closes it.
                app.dialog.as_mut().unwrap().tags.as_mut().unwrap().action = None;
                app.dispatch(Intent::View(View::Changes));
                assert!(app.dialog.is_none());
                assert_eq!(app.page(), None);

                // Ordinary modal dialogs still block input.
                app.dispatch(Intent::Dialog(DialogKind::Open));
                assert!(app.blocking_dialog());
                assert_eq!(app.page(), None);
            },
        );
    }

    /// Dialogs reuse one modal ID for content of different heights (Push, then
    /// Delete branch; an error that clears). The modal must shrink with it.
    #[test]
    fn review_shrinks_with_its_content() {
        use super::{Part, ReviewHeader, review};
        let ctx = Context::default();
        crate::style::install(&ctx);
        let mut widgets = std::collections::HashMap::new();
        let mut height = |lines: usize| {
            let mut measured = 0.0;
            // The first frame is egui's invisible sizing pass; let the size settle.
            for _ in 0..4 {
                let input = RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1200.0, 900.0))),
                    ..Default::default()
                };
                let mut output = ctx.run_ui(input, |ui| {
                    let head = ReviewHeader {
                        image: crate::style::Icon::Tag,
                        title: "Review",
                        subtitle: "",
                        danger: false,
                    };
                    let response = review(
                        ui.ctx(),
                        "shrink-test",
                        head,
                        &mut widgets,
                        "shrink-scroll",
                        true,
                        |ui, part, _| match part {
                            Part::Body => {
                                for i in 0..lines {
                                    ui.label(format!("Line {i}"));
                                }
                            }
                            Part::Footer => {
                                let _ = ui.button("Done");
                            }
                        },
                    );
                    measured = response.modal.response.rect.height();
                });
                output.textures_delta.clear();
            }
            measured
        };
        let tall = height(12);
        let short = height(1);
        assert!(
            short < tall - 150.0,
            "the review kept its earlier height: {short} after {tall}"
        );
    }

    /// A pane's loading state sits in its middle, not its top-left corner; work under
    /// content stays a line in place.
    #[test]
    fn loading_states_center_in_their_pane_and_status_lines_stay_put() {
        use eframe::egui::{CentralPanel, Shape, Ui};
        let ctx = Context::default();
        crate::style::install(&ctx);
        let painted = |content: &dyn Fn(&mut Ui), text: &str| {
            let mut found = None;
            for _ in 0..3 {
                let input = RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))),
                    ..Default::default()
                };
                let mut output = ctx.run_ui(input, |ui| {
                    CentralPanel::default().show(ui, |ui| content(ui));
                });
                found = output.shapes.iter().find_map(|s| match &s.shape {
                    Shape::Text(t) if t.galley.text() == text => Some(t.visual_bounding_rect()),
                    _ => None,
                });
                output.textures_delta.clear();
            }
            found.unwrap_or_else(|| panic!("{text} was not painted"))
        };
        let pane = painted(&|ui| super::loading(ui, "Loading diff…"), "Loading diff…");
        assert!((pane.center().x - 400.0).abs() < 4.0, "{pane:?}");
        assert!((pane.center().y - 300.0).abs() < 40.0, "{pane:?}");
        let alone = painted(&|ui| super::waiting(ui, "Reading"), "Reading");
        assert!((alone.center().x - 400.0).abs() < 4.0 && alone.top() > 40.0, "{alone:?}");
        let under = painted(
            &|ui| {
                ui.label("Above");
                super::waiting(ui, "Reading");
            },
            "Reading",
        );
        assert!(under.left() < 100.0 && under.top() < 60.0, "{under:?}");
    }

    /// Learn more follows the last word of its text, on that line and at the text's
    /// size; no button padding pushes it below the line or away from the word.
    #[test]
    fn learn_more_follows_the_last_word_of_its_text() {
        use super::paragraph_with_link;
        use eframe::egui::{Color32, FontId};
        let ctx = Context::default();
        crate::style::install(&ctx);
        let mut widgets = std::collections::HashMap::new();
        let about = "Save tracked changes, switch to an existing local branch, then restore staging and working changes. The recovery stash is kept. Untracked and ignored files stay in place; overwriting them is refused.";
        let (mut shown, mut line) = (None, 0.0);
        // The first frame is egui's invisible sizing pass.
        for _ in 0..2 {
            let input = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(600.0, 400.0))),
                ..Default::default()
            };
            let mut output = ctx.run_ui(input, |ui| {
                let font = FontId::proportional(14.0);
                line = ui.painter().layout_no_wrap("Learn more".into(), font, Color32::GRAY).size().y;
                shown = Some(paragraph_with_link(ui, &[about], 14.0, Color32::GRAY, true, &mut widgets, "guide-link"));
            });
            output.textures_delta.clear();
        }
        let (text, link) = shown.unwrap();
        let link = link.expect("the link is shown");
        assert!(text.rect.height() > 2.0 * line, "the text wraps: {:?}", text.rect);
        assert!((link.rect.height() - line).abs() < 1.0, "no button padding: {:?}", link.rect);
        assert!(
            (link.rect.bottom() - text.rect.bottom()).abs() < 1.0,
            "on the text's last line: {:?} {:?}",
            link.rect,
            text.rect
        );
        assert!(link.rect.left() > text.rect.left() + 1.0, "after the last word, not under it");
    }

    /// The header's × closes a review like Esc, and stays distinct from a click
    /// outside, which Bisect and Recovery ignore while they resize.
    #[test]
    fn the_header_close_button_closes_a_review() {
        use super::{Part, ReviewHeader, review};
        use crate::app::UiControl;
        use eframe::egui::{PointerButton, pos2};
        use std::collections::HashMap;
        fn frame(ctx: &Context, widgets: &mut HashMap<String, UiControl>, events: Vec<Event>) -> (bool, bool) {
            let mut seen = (false, false);
            let input = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1200.0, 900.0))),
                events,
                ..Default::default()
            };
            let mut output = ctx.run_ui(input, |ui| {
                let head = ReviewHeader {
                    image: crate::style::Icon::Tag,
                    title: "Review",
                    subtitle: "",
                    danger: false,
                };
                let response = review(ui.ctx(), "close-test", head, widgets, "close-scroll", true, |ui, part, _| {
                    if part == Part::Body {
                        ui.label("Body");
                    }
                });
                seen = (response.should_close(), response.close_clicked());
            });
            output.textures_delta.clear();
            seen
        }
        let click = |at: Pos2| {
            let button = |pressed| Event::PointerButton {
                pos: at,
                button: PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            };
            [vec![Event::PointerMoved(at), button(true)], vec![button(false)]]
        };
        let ctx = Context::default();
        crate::style::install(&ctx);
        let mut widgets = HashMap::new();
        // The first frame is egui's invisible sizing pass.
        for _ in 0..3 {
            assert_eq!(frame(&ctx, &mut widgets, vec![]), (false, false));
        }
        let [press, release] = click(widgets["dialog-close"].rect.center());
        frame(&ctx, &mut widgets, press);
        assert_eq!(frame(&ctx, &mut widgets, release), (true, true), "the × closes");
        let [press, release] = click(pos2(12.0, 12.0));
        frame(&ctx, &mut widgets, press);
        assert_eq!(frame(&ctx, &mut widgets, release), (true, false), "a click outside is not the ×");
    }

    /// The × also carries `dialog-cancel`, the name automation gave a footer Cancel;
    /// a footer Stop drawn after it takes the name.
    #[test]
    fn the_header_close_button_is_dialog_cancel_unless_the_footer_takes_it() {
        use super::{Part, ReviewHeader, control, review};
        use crate::app::UiControl;
        use std::collections::HashMap;
        fn frame(ctx: &Context, widgets: &mut HashMap<String, UiControl>, stop: bool) {
            let input = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1200.0, 900.0))),
                ..Default::default()
            };
            let mut output = ctx.run_ui(input, |ui| {
                let head = ReviewHeader {
                    image: crate::style::Icon::Tag,
                    title: "Review",
                    subtitle: "",
                    danger: false,
                };
                review(ui.ctx(), "cancel-test", head, widgets, "cancel-scroll", true, |ui, part, widgets| {
                    if part == Part::Footer && stop {
                        let r = ui.button("Stop");
                        control(widgets, "dialog-cancel", &r, ui.clip_rect());
                    }
                });
            });
            output.textures_delta.clear();
        }
        let ctx = Context::default();
        crate::style::install(&ctx);
        let mut widgets = HashMap::new();
        // The first frame is egui's invisible sizing pass.
        for _ in 0..3 {
            frame(&ctx, &mut widgets, false);
        }
        assert_eq!(widgets["dialog-cancel"].rect, widgets["dialog-close"].rect, "the × is dialog-cancel");
        frame(&ctx, &mut widgets, true);
        let stop = widgets["dialog-cancel"].rect;
        assert!(stop.top() > widgets["dialog-close"].rect.bottom(), "the footer's Stop keeps it: {stop:?}");
    }

    #[test]
    fn browsing_page_publishes_its_details_scroll_viewport() {
        common::isolated(
            "ROCKET_PAGES_UI_CASE",
            "pages::tests::browsing_page_publishes_its_details_scroll_viewport",
            || {
                let fixture = common::Fixture::new();
                fixture.write("file", "first");
                fixture.commit("first");
                fixture.run(&["tag", "v1"]);
                let ctx = Context::default();
                let mut app = Rocket::with_log_directory(
                    &ctx,
                    vec![fixture.repository.clone()],
                    fixture.root.join("logs"),
                );
                let deadline = Instant::now() + Duration::from_secs(40);
                let wait = |app: &mut Rocket| {
                    while app.busy() {
                        app.poll(&ctx);
                        assert!(Instant::now() < deadline);
                        thread::sleep(Duration::from_millis(10));
                    }
                };
                wait(&mut app);
                app.auto_refresh = false;
                app.dispatch(Intent::Dialog(DialogKind::Tags));
                wait(&mut app);
                let size = vec2(780.0, 620.0);
                let frame = |app: &mut Rocket| {
                    app.frame_widgets.clear();
                    let mut output = ctx.run_ui(
                        RawInput {
                            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
                            ..Default::default()
                        },
                        |ui| app.shell(ui, &mut Vec::new()),
                    );
                    output.textures_delta.clear();
                };
                for _ in 0..3 {
                    frame(&mut app);
                }
                let card = app
                    .frame_widgets
                    .get("page-details-scroll")
                    .expect("a browsing page publishes its details viewport")
                    .rect;
                assert!(card.height() > 0.0 && card.bottom() <= size.y, "{card:?}");

                // Under a review the inert page publishes none of its controls.
                app.dialog.as_mut().unwrap().tags.as_mut().unwrap().action =
                    Some(rocket_git::TagAction::Create);
                for _ in 0..2 {
                    frame(&mut app);
                }
                assert!(!app.frame_widgets.contains_key("page-details-scroll"));
            },
        );
    }
}

#[cfg(test)]
mod button_label_tests {
    use super::*;

    fn texts(shape: &Shape, shown: &mut Vec<String>) {
        match shape {
            Shape::Vec(shapes) => shapes.iter().for_each(|s| texts(s, shown)),
            Shape::Text(text) => shown.push(text.galley.text().to_owned()),
            _ => {}
        }
    }

    #[test]
    fn page_buttons_show_labels_without_a_trailing_ellipsis() {
        let ctx = Context::default();
        crate::style::install(&ctx);
        let mut output = ctx.run_ui(Default::default(), |ui| {
            primary(ui, true, "Review track…");
            danger(ui, true, "Delete branch…");
            danger_ghost(ui, true, "Review deletion…");
            ghost(ui, true, "Remote tags…");
            choice(ui, true, false, "Remote…");
        });
        output.textures_delta.clear();
        let mut shown = Vec::new();
        for clipped in &output.shapes {
            texts(&clipped.shape, &mut shown);
        }
        assert_eq!(
            shown,
            [
                "Review track",
                "Delete branch",
                "Review deletion",
                "Remote tags",
                "Remote"
            ]
        );
    }
}

#[cfg(test)]
mod split_tests {
    use super::*;

    // An open drawer or a long notice can leave a page less room than `split`'s
    // minimum height; the card's last action must still scroll into view.
    #[test]
    fn card_action_scrolls_into_view_when_the_split_overflows_the_page() {
        let ctx = Context::default();
        crate::style::install(&ctx);
        let size = vec2(900.0, 200.0);
        let mut widgets = HashMap::new();
        for _ in 0..4 {
            widgets.clear();
            let mut output = ctx.run_ui(
                RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
                    ..Default::default()
                },
                |ui| {
                    ui.add_space(size.y - 30.0);
                    split(ui, |ui, side| {
                        if side == Side::Details {
                            for line in 0..6 {
                                ui.label(format!("Field {line}"));
                            }
                            let r = primary(ui, true, "Review checkout");
                            control(&mut widgets, "card-action", &r, ui.clip_rect());
                            r.scroll_to_me_animation(
                                Some(Align::Max),
                                eframe::egui::style::ScrollAnimation::none(),
                            );
                        }
                    });
                },
            );
            output.textures_delta.clear();
        }
        let action = widgets
            .get("card-action")
            .expect("the card's last action scrolls into the shown part of the page");
        assert!(
            action.rect.height() > 0.0 && action.rect.bottom() <= size.y,
            "{:?}",
            action.rect
        );
    }
}
