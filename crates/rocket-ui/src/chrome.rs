//! Shared window chrome. Repository content and operations stay in their own views.
//!
//! Layout, top to bottom: menu bar, repository tabs, a Git-only toolbar owned by the
//! active tab, then sidebar + content, and a status bar. Panes are flat fills separated
//! by hairlines; selection is one accent bar.
use crate::pages::Page;
use crate::{
    app::*,
    backend::Action,
    style::{self, *},
    views,
};
use eframe::egui::*;
use rocket_git::*;

/// Below this window width the sidebar starts as an icon rail unless labels were
/// turned on explicitly for the session.
const NARROW_SIDEBAR: f32 = 980.0;

/// The status bar's light for background reading. It appears only for reading that
/// lasts a moment and stays on through short gaps, so back-to-back reads show as one
/// instead of a light blinking on and off for each step.
#[derive(Clone, Copy, Default)]
struct BusyIndicator {
    /// Start of the current stretch of work, in egui seconds.
    since: Option<f64>,
    /// When work was last seen running.
    last: f64,
    shown: bool,
    label: &'static str,
}

impl BusyIndicator {
    const SHOW_AFTER: f64 = 0.25;
    const HOLD: f64 = 0.4;

    /// Whether the spinner shows now, and after how many seconds that can change
    /// without new input.
    fn update(&mut self, working: Option<&'static str>, now: f64) -> (bool, Option<f64>) {
        if let Some(label) = working {
            let since = *self.since.get_or_insert(now);
            self.last = now;
            self.label = label;
            self.shown |= now - since >= Self::SHOW_AFTER;
            (
                self.shown,
                (!self.shown).then_some(since + Self::SHOW_AFTER - now),
            )
        } else if self.shown && now - self.last < Self::HOLD {
            (true, Some(self.last + Self::HOLD - now))
        } else {
            *self = Self::default();
            (false, None)
        }
    }
}

/// The context menu of a repository tab: show its folder, open its terminal or close it.
fn repository_menu(
    response: &Response,
    tab: &Tab,
    widgets: &mut std::collections::HashMap<String, UiControl>,
    intents: &mut Vec<Intent>,
) {
    response.context_menu(|ui| {
        // The working folder once known, otherwise the path the tab was opened with.
        let folder = tab
            .snapshot
            .as_ref()
            .and_then(|s| s.info.worktree.clone())
            .unwrap_or_else(|| tab.path.clone());
        let explorer = ui.button("Open in File Explorer");
        widgets.insert("tab-menu-explorer".into(), (&explorer).into());
        if explorer.clicked() {
            intents.push(Intent::ShowFolder(folder));
            ui.close();
        }
        let terminal = ui.button("Open in Terminal");
        widgets.insert("tab-menu-terminal".into(), (&terminal).into());
        if terminal.clicked() {
            intents.push(Intent::TerminalFor(tab.id));
            ui.close();
        }
        ui.separator();
        // Ctrl+W closes the active tab, which this one may not be.
        let close = ui.button("Close repository");
        widgets.insert("tab-menu-close".into(), (&close).into());
        if close.clicked() {
            intents.push(Intent::CloseTab(tab.id));
            ui.close();
        }
    });
}

impl Rocket {
    pub(crate) fn sidebar_rail(&self, width: f32) -> bool {
        !self.sidebar_labels || (width < NARROW_SIDEBAR && !self.sidebar_labels_narrow)
    }

    pub(crate) fn toggle_sidebar_labels(&mut self, ctx: &Context) {
        let narrow = ctx.content_rect().width() < NARROW_SIDEBAR;
        if self.sidebar_rail(ctx.content_rect().width()) {
            self.sidebar_labels = true;
            self.sidebar_labels_narrow = narrow;
        } else {
            self.sidebar_labels = false;
            self.sidebar_labels_narrow = false;
        }
        self.show_sidebar = true;
    }

    /// Diagnostics and the repository manager replace the content area and hide the
    /// bottom drawer. MCP and external tool settings live in Preferences.
    fn overlay(&self) -> bool {
        self.activity_history.open || self.page() == Some(Page::RepositoryManager)
    }

    /// The drawer is open on Activity, where the running operation has its "now" row.
    pub(crate) fn activity_row_visible(&self) -> bool {
        self.show_console && !self.drawer_collapsed && !self.overlay()
    }

    /// The running operation came from the open dialog, whose footer shows its
    /// progress and Stop. The status bar and Activity then only name it, so one
    /// operation has one spinner and one Stop.
    pub(crate) fn operation_in_dialog(&self) -> bool {
        self.write.is_some()
            && self.dialog.as_ref().is_some_and(|d| {
                d.signing.as_ref().is_some_and(|s| s.applying)
                    || d.commit.as_ref().is_some_and(|s| s.applying)
                    || d.merge.as_ref().is_some_and(|s| s.applying)
                    || d.pull.as_ref().is_some_and(|s| s.fetching || s.applying)
                    || d.force_push.as_ref().is_some_and(|s| s.applying)
                    || d.atomic_push.as_ref().is_some_and(|s| s.applying)
                    || d.ignore.as_ref().is_some_and(|s| s.applying)
                    || d.workflow.as_ref().is_some_and(|s| s.applying)
                    || d.rebase.as_ref().is_some_and(|s| s.applying)
                    || d.bisect.as_ref().is_some_and(|s| s.applying)
                    || d.recovery.as_ref().is_some_and(|s| s.applying)
                    || d.tags.as_ref().is_some_and(|s| s.applying)
                    || d.worktrees.as_ref().is_some_and(|s| s.applying)
                    || d.submodules.as_ref().is_some_and(|s| s.applying)
                    || d.lfs.as_ref().is_some_and(|s| s.applying)
                    || d.file_edit.as_ref().is_some_and(|s| s.applying)
                    || d.patch.as_ref().is_some_and(|s| s.applying)
                    || d.gitflow.as_ref().is_some_and(|s| s.applying)
            })
    }

    /// Reading that runs in the background, named for the status bar's light. Pages,
    /// dialogs and a repository's first read show their own reads where the data
    /// appears, and the running operation has its own indicator; none of those is here.
    fn background_reading(&self) -> Option<&'static str> {
        if self.quitting {
            return self.working().then_some("Closing");
        }
        // The active repository's first read is the content area's spinner.
        let refreshing = self.tabs.iter().enumerate().any(|(index, tab)| {
            tab.refresh.is_some() && (index != self.active || tab.snapshot.is_some())
        });
        if self.saving() {
            Some("Saving drafts and settings")
        } else if self.mcp.busy()
            && self.preferences_page() != Some(crate::preferences::Page::Mcp)
        {
            Some("Checking local server")
        } else if refreshing {
            Some("Refreshing")
        } else if self.tabs.iter().any(|t| t.more.is_some()) {
            Some("Loading history")
        } else if self.recent_heads.running() {
            Some("Reading recent repositories")
        } else if self.paste_patch.is_some() {
            Some("Reading the clipboard")
        } else if self.forge.running() && self.page() != Some(Page::Hosting) {
            Some("Waiting for the hosting service")
        } else if self.ssh.running() && self.preferences_page() != Some(crate::preferences::Page::SshKeys) {
            Some("Working with SSH keys")
        } else if self.manager.running() && self.page() != Some(Page::RepositoryManager) {
            Some("Opening repositories")
        } else {
            None
        }
    }

    pub(crate) fn shell(&mut self, ui: &mut Ui, intents: &mut Vec<Intent>) {
        let width = ui.available_width();
        let compact = width < 1120.0;
        let line = Stroke::new(1.0, HAIRLINE());
        // The newest result counts as seen once the Activity drawer shows it, so the
        // toolbar dot and the status-bar chip stop repeating what is on screen.
        if self.show_console
            && !self.overlay()
            && let Some(last) = self.activities.last_mut()
        {
            last.seen = true;
        }
        Panel::top("menubar")
            .exact_size(30.0)
            .frame(Frame::new().fill(PANE_DEEP()).inner_margin(Margin {
                left: 6,
                right: 8,
                top: 0,
                bottom: 0,
            }))
            .show(ui, |ui| self.menubar(ui, intents));
        let repository = !self.tabs.is_empty();
        // The Repositories list replaces the tab strip only while the full sidebar
        // shows; otherwise the tabs stay reachable above the toolbar.
        let list = self.repository_list
            && self.show_sidebar
            && !self.sidebar_rail(width)
            && !self.activity_history.open;
        if repository && !list {
            Panel::top("repository-tabs")
                .exact_size(38.0)
                .frame(Frame::new().fill(PANE_DEEP()).inner_margin(Margin {
                    left: 8,
                    right: 8,
                    top: 5,
                    bottom: 0,
                }))
                .show(ui, |ui| self.repository_tabs(ui, intents));
        }
        if repository {
            let rect = Panel::top("toolbar")
                .exact_size(48.0)
                .frame(
                    Frame::new()
                        .fill(PANE_PANEL())
                        .inner_margin(Margin::symmetric(8, 0)),
                )
                .show(ui, |ui| self.toolbar(ui, intents, compact))
                .response
                .rect;
            ui.painter()
                .hline(rect.x_range(), rect.bottom() - 0.5, line);
        }
        Panel::bottom("statusbar")
            .exact_size(26.0)
            .frame(
                Frame::new()
                    .fill(PANE_DEEP())
                    .inner_margin(Margin::symmetric(10, 0)),
            )
            .show(ui, |ui| self.statusbar(ui));
        let library_page = self.page() == Some(Page::RepositoryManager);
        let overlay = self.overlay();
        if self.show_console && !overlay {
            self.show_terminal = false;
        }
        if (self.show_console || self.show_terminal) && !overlay && self.drawer_collapsed {
            // Its own panel id, so the open drawer keeps the height it was dragged to.
            let activity = self.show_console;
            Panel::bottom("drawer-strip")
                .exact_size(36.0)
                .resizable(false)
                .frame(Frame::new().fill(PANE_DEEP()).inner_margin(0))
                .show(ui, |ui| self.drawer_header(ui, activity));
        } else if self.show_console && !overlay {
            Panel::bottom("activity")
                .default_size(220.0)
                .size_range(110.0..=420.0)
                .resizable(true)
                .frame(Frame::new().fill(PANE_DEEP()).inner_margin(0))
                .show(ui, |ui| self.activity(ui));
        } else if self.show_terminal && !overlay {
            // Leave room for the repository tabs, diff controls and selectable
            // lines when a tall terminal is resized into a short window.
            let maximum = (ui.available_height() - 300.0).clamp(150.0, 500.0);
            Panel::bottom("terminal-panel")
                .default_size(250.0)
                .size_range(150.0..=maximum)
                .resizable(true)
                .frame(Frame::new().fill(PANE_DEEP()).inner_margin(0))
                .show(ui, |ui| {
                    self.drawer_header(ui, false);
                    Frame::new()
                        .inner_margin(Margin::symmetric(12, 8))
                        .show(ui, |ui| self.terminal_panel(ui));
                });
        }
        if self.show_sidebar && repository && !overlay {
            let rail = self.sidebar_rail(width);
            let panel = if rail {
                Panel::left("sidebar-rail")
                    .exact_size(52.0)
                    .resizable(false)
            } else {
                Panel::left("sidebar")
                    .default_size(236.0)
                    .size_range(196.0..=340.0)
                    .resizable(true)
            };
            let rect = panel
                .frame(Frame::new().fill(PANE_PANEL()))
                .show(ui, |ui| {
                    if list && !rail {
                        self.repository_list(ui, intents);
                    }
                    self.sidebar(ui, intents, rail)
                })
                .response
                .rect;
            ui.painter().vline(rect.right() - 0.5, rect.y_range(), line);
        }
        let page = if library_page { Some(Page::RepositoryManager) } else if overlay { None } else { self.page() };
        let flat = page.is_some()
            || self.activity_history.open
            || !overlay
                && (self.tabs.is_empty()
                    || self.tab().is_some_and(|t| {
                        t.snapshot.is_some()
                            && matches!(
                                t.view,
                                View::Changes
                                    | View::Commits
                                    | View::Compare
                                    | View::Map
                                    | View::BranchReview
                                    | View::Branches
                                    | View::Remotes
                                    | View::Stashes
                                    | View::Tree
                                    | View::FileHistory
                                    | View::Conflict
                            )
                    }));
        let margin = if flat {
            Margin::ZERO
        } else {
            Margin::symmetric(14, 12)
        };
        CentralPanel::default().frame(Frame::new().fill(PANE_BG()).inner_margin(margin)).show(ui, |ui| {
            if self.activity_history.open {
                let current = self.backend.as_ref().map_or("", |b| b.log.session_id());
                self.activity_history.ui(ui, &mut self.frame_widgets, current); return;
            }
            if let Some(page) = page { self.page_ui(ui, page); return; }
            if self.setup.open { self.setup_view(ui, intents); return; }
            if self.tabs.is_empty() { self.welcome(ui, intents); return; }
            let enabled = self.write.is_none() && !self.quitting;
            let tab = &mut self.tabs[self.active];
            if let Some(error) = &tab.error { banner(ui, RED(), error); }
            if let Some(snapshot) = &tab.snapshot {
                for warning in &snapshot.warnings { banner(ui, AMBER(), warning); }
                if let Some(state) = &snapshot.state && !state.is_idle() {
                    if tab.view == View::Conflict && ui.available_height() < 300.0 {
                        ui.add(Label::new(RichText::new("Git operation in progress · recovery controls in Changes").color(AMBER())).truncate())
                            .on_hover_text("Save and stage the resolution, then return to Changes to continue, skip or abort the current operation.");
                    } else {
                        operation_banner(ui, state, snapshot.status.as_ref(), snapshot.sequence.as_ref(), enabled, intents, &mut self.frame_widgets);
                    }
                }
            }
            if tab.snapshot.is_none() {
                ui.centered_and_justified(|ui| {
                    if tab.loading() { ui.spinner(); }
                    else { ui.label("Repository unavailable. Check the path and refresh, or open another repository."); }
                });
                return;
            }
            let writable = enabled && !tab.loading() && tab.worktree();
            match tab.view {
                View::Summary => crate::guided::summary(ui, tab, enabled, intents, &mut self.frame_widgets),
                View::Conflict => crate::conflicts::view(ui, tab, writable, self.write.as_ref().filter(|w| w.owner == Some(tab.id)).map(|w| w.cancelling), &self.drafting, self.mcp.hosted || self.mcp.status.running, intents, &mut self.frame_widgets),
                View::FileHistory => crate::file_history::view(ui, tab, intents, &mut self.frame_widgets, &self.commands, true),
                View::Tree => crate::tree::view(ui, tab, intents, &mut self.frame_widgets, &self.commands),
                View::Map => crate::repository_map::view(ui, tab, intents, &mut self.frame_widgets),
                View::Compare => crate::compare::view(ui, tab, intents, &mut self.frame_widgets),
                View::BranchReview => crate::branch_review::view(ui, tab, &self.drafting, intents, &mut self.frame_widgets),
                View::Commits => views::commits(ui, tab, self.show_details, self.row_height, intents, &mut self.frame_widgets, &self.commands),
                View::Changes => views::changes(ui, tab, writable, &mut self.spelling, intents, &mut self.frame_widgets, &self.commands),
                View::Branches => views::branches(ui, tab, writable, intents, &mut self.frame_widgets, &self.commands),
                View::Remotes => views::remotes(ui, tab, enabled && !tab.loading(), intents, &mut self.frame_widgets),
                View::Stashes => crate::stashes::view(ui, tab, writable, intents, &mut self.frame_widgets),
            }
        });
    }

    /// Toolbar, Ctrl+`, View menu and palette: a hidden or collapsed drawer opens on
    /// Activity, an open one hides completely.
    pub(crate) fn toggle_activity(&mut self) {
        let show = self.activity_history.open || !self.show_console || self.drawer_collapsed;
        self.activity_history.close();
        self.show_console = show;
        if show {
            self.drawer_collapsed = false;
        }
    }

    /// Activity on request: the whole drawer, not just its strip.
    pub(crate) fn show_activity(&mut self) {
        self.show_console = true;
        self.drawer_collapsed = false;
        self.activity_history.close();
    }

    /// Terminal and Activity share one bottom drawer. Its header switches between them,
    /// holds the shown pane's actions and collapses the drawer to just this strip.
    pub(crate) fn drawer_header(&mut self, ui: &mut Ui, activity: bool) {
        let collapsed = self.drawer_collapsed;
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 36.0), Sense::hover());
        ui.painter().rect_filled(rect, 0.0, VIEW_PANEL());
        ui.painter().hline(
            rect.x_range(),
            rect.bottom() - 0.5,
            Stroke::new(1.0, HAIRLINE()),
        );
        let mut x = rect.left() + 6.0;
        let needs_review = self.needs_review();
        for (is_activity, label, image, key) in [
            (false, "Terminal", Icon::Console, "drawer-terminal"),
            (true, "Activity", Icon::Activity, "drawer-activity"),
        ] {
            let width = ui
                .painter()
                .layout_no_wrap(label.into(), semibold(13.0), TEXT())
                .size()
                .x
                + 44.0;
            let tab = Rect::from_min_size(pos2(x, rect.top()), vec2(width, 36.0));
            let response = ui.interact(tab, Id::new(key), Sense::click());
            let on = is_activity == activity;
            let color = if on || response.hovered() {
                TEXT()
            } else {
                MUTED()
            };
            icon(
                ui.painter(),
                Rect::from_center_size(pos2(tab.left() + 16.0, tab.center().y), vec2(14.0, 14.0)),
                image,
                color,
            );
            ui.painter().text(
                pos2(tab.left() + 30.0, tab.center().y),
                Align2::LEFT_CENTER,
                label,
                if on {
                    semibold(13.0)
                } else {
                    FontId::proportional(13.0)
                },
                color,
            );
            // Only while Activity is not the shown tab: viewing it marks the entry seen.
            let response = if is_activity && needs_review {
                ui.painter()
                    .circle_filled(pos2(tab.right() - 8.0, tab.center().y), 3.5, AMBER());
                response.on_hover_text("The last operation needs review")
            } else {
                response
            };
            if on && !collapsed {
                ui.painter().rect_filled(
                    Rect::from_min_max(
                        pos2(tab.left() + 8.0, tab.bottom() - 2.0),
                        pos2(tab.right() - 8.0, tab.bottom()),
                    ),
                    1.0,
                    BLUE(),
                );
            }
            response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, label));
            self.frame_widgets.insert(key.into(), (&response).into());
            // A tab opens a collapsed drawer as well as switching to its pane.
            if response.clicked() {
                if is_activity {
                    self.show_activity();
                } else {
                    self.show_console = false;
                    self.show_terminal = true;
                    self.drawer_collapsed = false;
                }
            }
            x = tab.right() + 2.0;
        }
        // Right of the tabs: the pane's actions, or the collapsed line, then the arrow.
        // Each action keeps one size when hovered, so its neighbours never move.
        let actions = Rect::from_min_max(
            pos2(x + 6.0, rect.top() + 4.0),
            pos2(rect.right() - 8.0, rect.top() + 32.0),
        );
        ui.scope_builder(
            UiBuilder::new()
                .max_rect(actions)
                .layout(Layout::right_to_left(Align::Center)),
            |ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                let (image, hover, name, key) = if collapsed {
                    (Icon::ChevronUp, "Expand", "Expand drawer", "drawer-expand")
                } else {
                    (Icon::Chevron, "Collapse", "Collapse drawer", "drawer-collapse")
                };
                let arrow = icon_button(ui, image, true, false, 26.0).on_hover_text(hover);
                arrow.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, name));
                self.frame_widgets.insert(key.into(), (&arrow).into());
                if !collapsed {
                    // The ids of the old hide buttons, which automation still uses.
                    let old = if activity { "console-close" } else { "terminal-hide" };
                    self.frame_widgets.insert(old.into(), (&arrow).into());
                }
                if arrow.clicked() {
                    self.drawer_collapsed = !collapsed;
                }
                let (line, _) = ui.allocate_exact_size(vec2(9.0, 18.0), Sense::hover());
                ui.painter()
                    .vline(line.center().x, line.y_range(), Stroke::new(1.0, HAIRLINE()));
                if collapsed {
                    self.drawer_summary(ui, activity);
                } else if activity {
                    self.activity_actions(ui);
                } else {
                    self.terminal_actions(ui);
                }
            },
        );
    }

    /// The collapsed drawer's one line: the running or last operation on Activity, the
    /// shell's state on Terminal. Clicking it opens the drawer again.
    fn drawer_summary(&mut self, ui: &mut Ui, activity: bool) {
        let (dot, title, detail) = if activity {
            if let Some(write) = &self.write {
                (Some(BLUE()), write.title.clone(), write.progress.clone())
            } else if let Some(last) = self.activities.last() {
                let outcome = &last.outcome;
                let color = if outcome.paused {
                    AMBER()
                } else if !outcome.command_succeeded {
                    RED()
                } else if outcome.recovery_pending {
                    AMBER()
                } else {
                    GREEN()
                };
                let age = std::time::SystemTime::now()
                    .duration_since(last.at)
                    .unwrap_or_default()
                    .as_secs();
                let unix = last
                    .at
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.as_secs() as i64);
                let when = ago(age, unix);
                (
                    Some(color),
                    crate::operation_names::done(&last.title),
                    format!("{} · {when}", outcome.summary),
                )
            } else {
                (None, String::new(), "No operations yet".to_owned())
            }
        } else {
            match self.terminal_state() {
                Some((running, state)) => {
                    (Some(if running { GREEN() } else { FAINT() }), self.terminal_session_name(), state)
                }
                None => (None, String::new(), "No terminal session".to_owned()),
            }
        };
        let (area, response) =
            ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::click());
        if response.hovered() || response.has_focus() {
            ui.painter().rect(
                area,
                6.0,
                HOVER(),
                Stroke::new(1.0, EDGE_HOT()),
                StrokeKind::Inside,
            );
        }
        let painter = ui.painter_at(area);
        let mut left = area.left() + 10.0;
        if let Some(color) = dot {
            painter.circle_filled(pos2(left + 3.5, area.center().y), 3.5, color);
            left += 15.0;
        }
        if !title.is_empty() {
            let galley = painter.layout_no_wrap(title.clone(), semibold(12.5), TEXT());
            let width = galley.size().x;
            painter.galley(pos2(left, area.center().y - galley.size().y / 2.0), galley, TEXT());
            left += width + 8.0;
        }
        let mut job = text::LayoutJob::simple_singleline(
            detail.clone(),
            FontId::proportional(12.5),
            MUTED(),
        );
        job.wrap = text::TextWrapping::truncate_at_width((area.right() - 10.0 - left).max(0.0));
        let galley = painter.layout_job(job);
        painter.galley(pos2(left, area.center().y - galley.size().y / 2.0), galley, MUTED());
        let line = if title.is_empty() { detail } else { format!("{title} · {detail}") };
        let response = response.on_hover_text(&line);
        response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, &line));
        self.frame_widgets
            .insert("drawer-summary".into(), (&response).into());
        if response.clicked() {
            self.drawer_collapsed = false;
        }
    }

    /// The newest result failed or needs recovery and the Activity drawer has not
    /// shown it yet.
    fn needs_review(&self) -> bool {
        self.activities.last().is_some_and(|a| {
            !a.seen && (!a.outcome.command_succeeded || a.outcome.recovery_pending)
        })
    }

    fn statusbar(&mut self, ui: &mut Ui) {
        let mut dismiss_notice = false;
        ui.spacing_mut().interact_size.y = 20.0;
        ui.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            if let Some(tab) = self.tab() {
                small_icon(ui, Icon::Branch, FAINT());
                ui.label(RichText::new(tab.branch_label()).size(12.0).color(MUTED()));
                if let Some((ahead, behind)) = tab
                    .snapshot
                    .as_ref()
                    .and_then(|s| s.status.as_ref())
                    .and_then(|s| s.ahead_behind)
                {
                    ui.label(
                        RichText::new(format!("↑{ahead}"))
                            .size(12.0)
                            .color(if ahead > 0 { BLUE() } else { FAINT() }),
                    );
                    ui.label(
                        RichText::new(format!("↓{behind}"))
                            .size(12.0)
                            .color(if behind > 0 { MUTED() } else { FAINT() }),
                    );
                }
            }
            self.auto_fetch_status(ui);
            if self.write.is_none() && ui.available_width() > 480.0 {
                self.dependency_status(ui);
            }
            // The running operation is loud here (spinner, progress and Stop) only when
            // neither its dialog nor the Activity drawer shows it; otherwise it is named.
            let quiet = self.operation_in_dialog() || self.activity_row_visible();
            if let Some(write) = &self.write
                && quiet
            {
                ui.add_space(6.0);
                let (dot, _) = ui.allocate_exact_size(vec2(7.0, 7.0), Sense::hover());
                ui.painter().circle_filled(dot.center(), 3.5, BLUE());
                ui.add(Label::new(RichText::new(&write.title).size(12.0).color(MUTED())).truncate())
                    .on_hover_text(format!("{} · {}", write.title, write.progress));
            } else if let Some(write) = &self.write {
                ui.add_space(6.0);
                ui.spinner();
                // Progress can change between hit-testing and this layout. Keep
                // Stop ahead of it, with one identity for the lifetime of the write.
                // Otherwise a new progress bar changes its auto ID and changing
                // text moves its hit rectangle while the click is being delivered.
                let cancel = ui
                    .push_id(("status-stop", write.started), |ui| {
                        ui.spacing_mut().button_padding = vec2(8.0, 1.0);
                        ui.add_enabled(
                            write.can_cancel(),
                            Button::new(RichText::new(write.cancel_label()).size(12.0).color(RED()))
                                .fill(Color32::TRANSPARENT)
                                .stroke(Stroke::new(1.0, RED().gamma_multiply(0.45))),
                        )
                    })
                    .inner;
                ui.add(
                    Label::new(
                        RichText::new(format!("{} · {}", write.title, write.progress))
                            .size(12.0)
                            .color(BLUE()),
                    )
                    .truncate(),
                );
                if let Some(fraction) = write.fraction {
                    ui.add(
                        ProgressBar::new(fraction)
                            .desired_width(90.0)
                            .desired_height(4.0)
                            .fill(BLUE())
                            .corner_radius(2),
                    );
                }
                self.frame_widgets
                    .insert("status-cancel".into(), (&cancel).into());
                if cancel.clicked() {
                    self.cancel_write();
                }
            } else if let Some(notice) = &self.notice {
                ui.add_space(6.0);
                let width = (ui.available_width() - 360.0).max(80.0);
                ui.scope(|ui| {
                    ui.set_max_width(width);
                    ui.add(Label::new(RichText::new(notice).size(12.0).color(MUTED())).truncate())
                        .on_hover_text(notice);
                });
                dismiss_notice = ui
                    .add(crate::style::quiet(Button::new(
                        RichText::new("×").size(12.0).color(FAINT()),
                    )))
                    .on_hover_text("Dismiss status message")
                    .clicked();
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                self.update_status_ui(ui);
                if let Some(backend) = &self.backend {
                    // Reading that is actually running, not a save waiting out its
                    // debounce, and only what nothing else on screen shows.
                    let reading = self.background_reading();
                    let id = Id::new("status-busy");
                    let mut indicator: BusyIndicator =
                        ui.data(|d| d.get_temp(id)).unwrap_or_default();
                    let now = ui.input(|i| i.time);
                    let (busy, recheck) = indicator.update(reading, now);
                    ui.data_mut(|d| d.insert_temp(id, indicator));
                    if let Some(after) = recheck {
                        ui.ctx()
                            .request_repaint_after(std::time::Duration::from_secs_f64(
                                after.max(0.0),
                            ));
                    }
                    if busy {
                        ui.label(RichText::new(indicator.label).size(12.0).color(MUTED()))
                            .on_hover_text(&backend.version);
                        // A small light that fades between full and 30% every 1.6 s.
                        let (light, _) = ui.allocate_exact_size(vec2(13.0, 13.0), Sense::hover());
                        let phase = (now / 1.6).fract() as f32;
                        let alpha = 0.65 + 0.35 * (phase * std::f32::consts::TAU).cos();
                        ui.painter()
                            .circle_filled(light.center(), 6.5, BLUE().gamma_multiply(0.16 * alpha));
                        ui.painter()
                            .circle_filled(light.center(), 3.5, BLUE().gamma_multiply(alpha));
                        ui.ctx()
                            .request_repaint_after(std::time::Duration::from_millis(50));
                    } else if self.write.is_none() {
                        let state = if self.tab().is_some_and(|t| t.snapshot.is_some()) {
                            "Repository ready"
                        } else {
                            "Ready"
                        };
                        ui.label(RichText::new(state).size(12.0).color(MUTED()))
                            .on_hover_text(&backend.version);
                        small_icon(ui, Icon::Check, FAINT());
                    }
                }
                let running = self.mcp.status.running;
                let mcp = chip(ui, Icon::Plug, "MCP", false, None).on_hover_text(if running {
                    "MCP server running · open its settings"
                } else {
                    "MCP server settings"
                });
                if running {
                    ui.painter()
                        .circle_filled(mcp.rect.right_top() + vec2(-5.0, 5.0), 3.0, GREEN());
                }
                self.frame_widgets
                    .insert("toolbar-mcp".into(), (&mcp).into());
                // Existing local controllers keep their old target names.
                self.frame_widgets.insert("tab-mcp".into(), (&mcp).into());
                if mcp.clicked() {
                    self.show_preferences(crate::preferences::Page::Mcp);
                }
                // A hidden external tool keeps running; this brings its dialog back.
                if !self.external.open && (self.external.running() || self.external.report.is_some()) {
                    let label = if self.external.running() { "Tool running" } else { "Tool finished" };
                    let tool = chip(ui, Icon::External, label, false, Some(BLUE()))
                        .on_hover_text("Show the external tool");
                    self.frame_widgets.insert("status-tool".into(), (&tool).into());
                    if tool.clicked() {
                        self.external.open = true;
                    }
                }
                if self.needs_review() {
                    let review = chip(ui, Icon::Activity, "Needs review", false, Some(AMBER()))
                        .on_hover_text(
                            "The last operation needs attention. Open Activity for details.",
                        );
                    self.frame_widgets
                        .insert("status-review".into(), (&review).into());
                    if review.clicked() {
                        self.show_activity();
                    }
                }
                if let Some(error) = &self.state_error {
                    ui.label(
                        RichText::new("Drafts/settings not saved")
                            .size(12.0)
                            .color(AMBER()),
                    )
                    .on_hover_text(error);
                    if self.can_retry_state_save() {
                        let retry = chip(ui, Icon::Refresh, "Retry save", false, Some(AMBER()));
                        self.frame_widgets.insert("status-retry-save".into(), (&retry).into());
                        if retry.clicked() {
                            self.retry_state_save();
                        }
                    }
                }
                if self
                    .backend
                    .as_ref()
                    .is_some_and(|b| b.log.health().dropped_events > 0)
                {
                    ui.label(
                        RichText::new("Diagnostic events dropped")
                            .size(12.0)
                            .color(AMBER()),
                    );
                }
            });
        });
        if dismiss_notice {
            self.notice = None;
        }
    }

    fn repository_tabs(&mut self, ui: &mut Ui, intents: &mut Vec<Intent>) {
        ScrollArea::horizontal()
            .id_salt("tabs-scroll")
            .auto_shrink([false, true])
            .scroll_bar_visibility(scroll_area::ScrollBarVisibility::AlwaysHidden)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    for (i, tab) in self.tabs.iter().enumerate() {
                        // The accessible name keeps the unsaved-draft marker.
                        let title = format!(
                            "{}{}",
                            tab.title,
                            if tab.draft.is_empty() { "" } else { "*" }
                        );
                        let selected = i == self.active && !self.activity_history.open;
                        let dirty = tab
                            .snapshot
                            .as_ref()
                            .and_then(|s| s.status.as_ref())
                            .is_some_and(|s| !s.entries.is_empty());
                        let branch = (!selected).then(|| tab.branch_label());
                        let name_font = if selected {
                            semibold(13.0)
                        } else {
                            FontId::proportional(13.0)
                        };
                        let name_width = ui
                            .painter()
                            .layout_no_wrap(tab.title.clone(), name_font.clone(), TEXT())
                            .size()
                            .x;
                        let branch_width = branch.as_ref().map_or(0.0, |b| {
                            ui.painter()
                                .layout_no_wrap(b.clone(), FontId::proportional(12.0), FAINT())
                                .size()
                                .x
                                + 24.0
                        });
                        let lead = if dirty { 26.0 } else { 12.0 };
                        let width = (lead + name_width + branch_width + 40.0).clamp(130.0, 260.0);
                        let (rect, _) = ui.allocate_exact_size(vec2(width, 33.0), Sense::hover());
                        let close_rect = Rect::from_center_size(
                            pos2(rect.right() - 16.0, rect.center().y),
                            vec2(22.0, 22.0),
                        );
                        let response = ui.interact(
                            Rect::from_min_max(rect.min, pos2(close_rect.left(), rect.bottom())),
                            Id::new(("tab", tab.id)),
                            Sense::click(),
                        );
                        let close =
                            ui.interact(close_rect, Id::new(("close-tab", tab.id)), Sense::click());
                        let hovered = response.hovered() || close.hovered();
                        let top = CornerRadius {
                            nw: 8,
                            ne: 8,
                            sw: 0,
                            se: 0,
                        };
                        if selected {
                            ui.painter().rect_filled(rect, top, PANE_PANEL());
                        } else if hovered {
                            ui.painter()
                                .rect_filled(rect, top, TAB_HOT());
                        }
                        if dirty {
                            ui.painter().circle_filled(
                                pos2(rect.left() + 15.0, rect.center().y),
                                3.5,
                                AMBER(),
                            );
                        }
                        let text_right = close_rect.left() - 4.0;
                        let name_rect = Rect::from_min_max(
                            pos2(rect.left() + lead, rect.top()),
                            pos2(text_right - branch_width, rect.bottom()),
                        );
                        ui.painter().with_clip_rect(name_rect).text(
                            pos2(name_rect.left(), rect.center().y),
                            Align2::LEFT_CENTER,
                            &tab.title,
                            name_font,
                            if selected || hovered { TEXT() } else { MUTED() },
                        );
                        if let Some(branch) = &branch {
                            let left = name_rect.right() + 8.0;
                            icon(
                                ui.painter(),
                                Rect::from_center_size(
                                    pos2(left + 6.0, rect.center().y),
                                    vec2(11.0, 11.0),
                                ),
                                Icon::Branch,
                                FAINT(),
                            );
                            clipped_text(
                                ui,
                                Rect::from_min_max(
                                    pos2(left + 16.0, rect.top()),
                                    pos2(text_right, rect.bottom()),
                                ),
                                branch,
                                12.0,
                                FAINT(),
                            );
                        }
                        if selected || hovered || close.has_focus() {
                            if close.hovered() || close.has_focus() {
                                ui.painter()
                                    .rect_filled(close_rect.shrink(1.0), 4.0, RAISED());
                            }
                            let c = close_rect.center();
                            for delta in [-3.5, 3.5] {
                                ui.painter().line_segment(
                                    [c + vec2(-3.5, delta), c + vec2(3.5, -delta)],
                                    Stroke::new(1.2, if close.hovered() { TEXT() } else { FAINT() }),
                                );
                            }
                        }
                        response
                            .widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, &title));
                        close.widget_info(|| {
                            WidgetInfo::labeled(
                                WidgetType::Button,
                                true,
                                format!("Close {}", tab.title),
                            )
                        });
                        if response.clicked() {
                            self.active = i;
                            self.external.open = false;
                            self.activity_history.close();
                        }
                        if close.clicked() {
                            intents.push(Intent::CloseTab(tab.id));
                        }
                        repository_menu(&response, tab, &mut self.frame_widgets, intents);
                        self.frame_widgets
                            .insert(format!("tab-{}", tab.id), (&response).into());
                        self.frame_widgets
                            .insert(format!("close-tab-{}", tab.id), (&close).into());
                        response.on_hover_text(tab.path.to_string_lossy());
                        close.on_hover_text("Close repository (Ctrl+W)");
                    }
                    ui.add_space(4.0);
                    let add = icon_button(ui, Icon::Plus, true, false, 28.0)
                        .on_hover_text("Open or clone a repository (Ctrl+O)");
                    self.frame_widgets.insert("tab-add".into(), (&add).into());
                    Popup::menu(&add).show(|ui| self.open_menu(ui, intents));
                });
            });
    }

    /// Open, clone, create or reopen a repository: the menu of each + for repositories.
    fn open_menu(&mut self, ui: &mut Ui, intents: &mut Vec<Intent>) {
        if ui.button("Open repository").clicked() {
            intents.push(Intent::Dialog(DialogKind::Open));
        }
        if ui
            .add_enabled(self.write.is_none(), Button::new("Clone repository"))
            .clicked()
        {
            intents.push(Intent::Dialog(DialogKind::Clone));
        }
        let create = ui.add_enabled(
            self.backend.is_some() && self.write.is_none(),
            Button::new("New repository"),
        );
        self.frame_widgets
            .insert("toolbar-new-repository".into(), (&create).into());
        if create.clicked() {
            intents.push(Intent::Dialog(DialogKind::NewRepository));
        }
        let reopen = ui.add_enabled(
            !self.closed_tabs.is_empty(),
            Button::new("Reopen closed repository").shortcut_text(crate::shortcuts::hint(ui.ctx(), "Ctrl+Shift+T")),
        );
        self.frame_widgets
            .insert("tab-reopen".into(), (&reopen).into());
        if reopen.clicked() {
            intents.push(Intent::ReopenTab);
        }
    }

    /// Open repositories as sidebar rows, in place of the tab strip: the same
    /// switching, closing and context menu as the tabs.
    fn repository_list(&mut self, ui: &mut Ui, intents: &mut Vec<Intent>) {
        Frame::new()
            .inner_margin(Margin {
                left: 8,
                right: 8,
                top: 8,
                bottom: 4,
            })
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 1.0;
                let header = section_header(
                    ui,
                    "Repositories",
                    Some(self.tabs.len()),
                    "repositories",
                    false,
                    Some("Open or clone a repository"),
                    None,
                );
                if let Some(add) = &header.add {
                    self.frame_widgets
                        .insert("repositories-add".into(), add.into());
                    Popup::menu(add).show(|ui| self.open_menu(ui, intents));
                }
                if !header.expanded {
                    return;
                }
                for (i, tab) in self.tabs.iter().enumerate() {
                    let selected = i == self.active && !self.activity_history.open;
                    let (row, close) = repository_row(ui, tab, selected);
                    if row.clicked() {
                        self.active = i;
                        self.external.open = false;
                        self.activity_history.close();
                    }
                    if close.clicked() {
                        intents.push(Intent::CloseTab(tab.id));
                    }
                    self.frame_widgets
                        .insert(format!("repository-row-{}", tab.id), (&row).into());
                    self.frame_widgets
                        .insert(format!("repository-close-{}", tab.id), (&close).into());
                    repository_menu(&row, tab, &mut self.frame_widgets, intents);
                }
            });
        let line = ui
            .allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover())
            .0;
        ui.painter().hline(
            line.shrink2(vec2(10.0, 0.0)).x_range(),
            line.center().y,
            Stroke::new(1.0, HAIRLINE()),
        );
    }

    fn toolbar(&mut self, ui: &mut Ui, intents: &mut Vec<Intent>, compact: bool) {
        let writable = self.can_write() && !self.activity_history.open;
        let remote_ready = !self.activity_history.open
            && self.write.is_none()
            && self.tab().is_some_and(|t| {
                !t.loading() && t.snapshot.as_ref().is_some_and(|s| !s.remotes.is_empty())
            });
        let (ahead, behind) = self
            .tab()
            .and_then(|t| t.snapshot.as_ref())
            .and_then(|s| s.status.as_ref())
            .and_then(|s| s.ahead_behind)
            .unwrap_or((0, 0));
        let running = self.write.as_ref().map(|w| w.label.clone());
        ui.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing = vec2(4.0, 0.0);
            self.branch_picker(ui, intents, compact);
            let enabled = self.write.is_none() && self.tab().is_some_and(|t| !t.loading());
            let refresh = icon_button(ui, Icon::Refresh, enabled, false, 32.0)
                .on_hover_text("Refresh local state (F5)");
            refresh.widget_info(|| {
                WidgetInfo::labeled(WidgetType::Button, enabled, "Refresh local repository")
            });
            self.frame_widgets
                .insert("refresh".into(), (&refresh).into());
            if refresh.clicked() {
                intents.push(Intent::Refresh);
            }
            divider(ui);
            for (key, image, label, enabled, dialog, count, op, hint) in [
                (
                    "toolbar-fetch",
                    Icon::Fetch,
                    "Fetch",
                    remote_ready,
                    DialogKind::Fetch,
                    0,
                    "fetch",
                    "Download new commits from remotes",
                ),
                (
                    "toolbar-pull",
                    Icon::Pull,
                    "Pull",
                    writable,
                    DialogKind::Pull,
                    behind,
                    "pull",
                    "Bring remote commits into this branch",
                ),
                (
                    "toolbar-push",
                    Icon::Push,
                    "Push",
                    writable && remote_ready,
                    DialogKind::Push,
                    ahead,
                    "push",
                    "Upload this branch's commits",
                ),
            ] {
                let busy = running.as_deref() == Some(op);
                let response = tool_button(
                    ui,
                    image,
                    label,
                    ToolOptions {
                        enabled,
                        label: !compact,
                        count,
                        busy,
                        ..Default::default()
                    },
                )
                .on_hover_text(match count {
                    0 => hint.to_string(),
                    n => format!("{hint} · {n} commit{}", if n == 1 { "" } else { "s" }),
                });
                self.frame_widgets.insert(key.into(), (&response).into());
                if response.clicked() {
                    intents.push(Intent::Dialog(dialog));
                }
            }
            divider(ui);
            let stash = tool_button(
                ui,
                Icon::Stash,
                "Stash",
                ToolOptions {
                    enabled: true,
                    label: !compact,
                    chevron: true,
                    ..Default::default()
                },
            );
            self.frame_widgets
                .insert("toolbar-stash".into(), (&stash).into());
            Popup::menu(&stash).show(|ui| {
                if ui
                    .add_enabled(writable, Button::new("Save changes to stash"))
                    .clicked()
                {
                    intents.push(Intent::Dialog(DialogKind::Stash));
                    ui.close();
                }
                if ui
                    .add_enabled(self.tab().is_some(), Button::new("View stashes"))
                    .clicked()
                {
                    intents.push(Intent::View(View::Stashes));
                    ui.close();
                }
            });
            let branch = tool_button(
                ui,
                Icon::Branch,
                "Branch",
                ToolOptions {
                    enabled: writable,
                    label: !compact,
                    chevron: true,
                    ..Default::default()
                },
            )
            .on_hover_text("Branch actions: New, Sync and Finish");
            self.frame_widgets
                .insert("toolbar-branch".into(), (&branch).into());
            Popup::menu(&branch).show(|ui| {
                let new = crate::menus::item_response(ui, "New Branch", "Ctrl+Shift+B", writable);
                crate::pages::control(&mut self.frame_widgets, "toolbar-branch-new", &new, ui.clip_rect());
                if new.clicked() {
                    intents.push(Intent::Dialog(DialogKind::NewBranch));
                }
                ui.separator();
                let ready = self.can_branch_workflow();
                for (id, label, kind, hint) in [
                    ("toolbar-branch-sync", "Sync Branch", BranchWorkflowKind::Sync,
                        "Put this branch on top of a local base. Review saving and restoring your changes first."),
                    ("toolbar-branch-finish", "Finish Branch", BranchWorkflowKind::Finish,
                        "Merge into a local base. Review a merge commit or allow fast-forward."),
                ] {
                    let response = crate::menus::item_response(ui, label, "", ready)
                        .on_hover_text(hint)
                        .on_disabled_hover_text("Requires a local branch and no paused Git operation. Check recovery in Changes.");
                    crate::pages::control(&mut self.frame_widgets, id, &response, ui.clip_rect());
                    if response.clicked() {
                        intents.push(Intent::BranchWorkflow(kind));
                    }
                }
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                let review = self.needs_review();
                // Lit only while the drawer is open on that pane; a collapsed drawer
                // opens on the first click and hides on the second.
                let open = !self.drawer_collapsed;
                let activity = icon_button(ui, Icon::Activity, true, self.show_console && open, 32.0)
                    .on_hover_text(if review {
                        "Activity · the last operation needs review (Ctrl+`)"
                    } else {
                        "Activity (Ctrl+`)"
                    });
                if review {
                    ui.painter().circle(
                        activity.rect.right_top() + vec2(-8.0, 8.0),
                        3.5,
                        AMBER(),
                        Stroke::new(2.0, PANEL()),
                    );
                }
                self.frame_widgets
                    .insert("toolbar-console".into(), (&activity).into());
                if activity.clicked() {
                    self.toggle_activity();
                }
                let terminal = icon_button(
                    ui,
                    Icon::Console,
                    self.tab().is_some(),
                    self.show_terminal && open,
                    32.0,
                )
                .on_hover_text("Terminal (Ctrl+Shift+`)");
                self.frame_widgets
                    .insert("toolbar-terminal".into(), (&terminal).into());
                if terminal.clicked() {
                    if self.show_terminal && open {
                        self.show_terminal = false;
                    } else {
                        intents.push(Intent::Terminal);
                    }
                }
                ui.add_space(4.0);
                let search = command_field(ui, compact);
                self.frame_widgets
                    .insert("toolbar-palette".into(), (&search).into());
                if search.clicked() {
                    self.palette.show();
                }
            });
        });
    }

    fn branch_picker(&mut self, ui: &mut Ui, intents: &mut Vec<Intent>, compact: bool) {
        let Some(tab) = self.tabs.get(self.active) else {
            return;
        };
        let head = tab.branch_label();
        let name_width = ui
            .painter()
            .layout_no_wrap(head.clone(), semibold(13.0), TEXT())
            .size()
            .x;
        let width = if compact {
            (name_width + 66.0).clamp(110.0, 190.0)
        } else {
            (name_width + 76.0).clamp(190.0, 260.0)
        };
        let response = ui
            .add(Button::new("").frame(false).min_size(vec2(width, 38.0)))
            .on_hover_text(format!("Switch branch: {head}"));
        self.frame_widgets
            .insert("branch-switch".into(), (&response).into());
        let rect = response.rect;
        let painter = ui.painter();
        if response.hovered()
            || response.has_focus()
            || Popup::is_id_open(ui.ctx(), Popup::default_response_id(&response))
        {
            painter.rect_filled(rect, 8.0, HOVER());
        }
        let badge =
            Rect::from_center_size(pos2(rect.left() + 19.0, rect.center().y), vec2(28.0, 28.0));
        painter.rect_filled(badge, 7.0, RAISED());
        icon(painter, badge.shrink(7.0), Icon::Branch, BLUE());
        let text_left = badge.right() + 10.0;
        let text_clip = Rect::from_min_max(
            pos2(text_left, rect.top()),
            pos2(rect.right() - 22.0, rect.bottom()),
        );
        if compact {
            painter.with_clip_rect(text_clip).text(
                pos2(text_left, rect.center().y),
                Align2::LEFT_CENTER,
                &head,
                semibold(13.0),
                TEXT(),
            );
        } else {
            painter.text(
                pos2(text_left, rect.center().y - 1.0),
                Align2::LEFT_BOTTOM,
                "Current branch",
                FontId::proportional(10.5),
                FAINT(),
            );
            painter.with_clip_rect(text_clip).text(
                pos2(text_left, rect.center().y),
                Align2::LEFT_TOP,
                &head,
                semibold(13.0),
                TEXT(),
            );
        }
        icon(
            painter,
            Rect::from_center_size(pos2(rect.right() - 12.0, rect.center().y), vec2(11.0, 11.0)),
            Icon::Chevron,
            FAINT(),
        );
        response.widget_info(|| {
            WidgetInfo::labeled(WidgetType::Button, true, format!("Switch branch: {head}"))
        });
        let writable = self.can_write();
        Popup::menu(&response).show(|ui| {
            ui.label(RichText::new("SWITCH BRANCH").size(11.0).color(FAINT()));
            if let Some(snapshot) = &tab.snapshot {
                for branch in snapshot
                    .branches
                    .iter()
                    .filter(|b| b.kind == BranchKind::Local)
                {
                    let label = short_ref(&branch.reference);
                    let text = if branch.is_head {
                        format!("✓  {label}")
                    } else {
                        format!("     {label}")
                    };
                    if ui
                        .add_enabled(
                            self.write.is_none() && !tab.loading() && !branch.is_head,
                            Button::new(text),
                        )
                        .clicked()
                    {
                        intents.push(Intent::Execute(
                            tab.id,
                            Action::SwitchBranch(
                                branch
                                    .reference
                                    .strip_prefix(b"refs/heads/")
                                    .unwrap_or(&branch.reference)
                                    .to_vec(),
                            ),
                        ));
                        ui.close();
                    }
                }
            }
            ui.separator();
            if ui
                .add_enabled(writable, Button::new("New branch"))
                .clicked()
            {
                intents.push(Intent::Dialog(DialogKind::NewBranch));
                ui.close();
            }
        });
    }

    fn sidebar(&mut self, ui: &mut Ui, intents: &mut Vec<Intent>, rail: bool) {
        let blocked = self.blocking_dialog();
        let writable = self.can_write();
        let idle = self.write.is_none();
        // Background refresh and saving must not disable navigation to a page; a
        // write or a foreground load still does.
        let free = !self.interaction_busy();
        let page = self.page();
        let Some(tab) = self.tabs.get_mut(self.active) else {
            return;
        };
        // No view is selected while a management page covers the content area.
        let current = page.is_none().then_some(tab.view);
        let count = tab
            .snapshot
            .as_ref()
            .and_then(|s| s.status.as_ref())
            .map_or(0, |s| s.entries.len());
        let mut nav = |ui: &mut Ui, widgets: &mut std::collections::HashMap<String, UiControl>| {
            Frame::new()
                .inner_margin(Margin::symmetric(8, 8))
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 1.0;
                    for (view, label, image, key) in [
                        (View::Summary, "Summary", Icon::Folder, "view-summary"),
                        (View::Changes, "Changes", Icon::File, "view-changes"),
                        (View::Commits, "Commits", Icon::Commit, "view-commits"),
                        (View::Compare, "Compare", Icon::Branch, "view-compare"),
                        (View::Map, "Map", Icon::Branch, "view-map"),
                        (View::BranchReview, "AI review", Icon::Search, "view-review"),
                        (View::Tree, "Files", Icon::Folder, "view-tree"),
                    ] {
                        let response = nav_row(
                            ui,
                            label,
                            image,
                            current == Some(view),
                            true,
                            (view == View::Changes && count > 0).then_some(count),
                            rail,
                        );
                        widgets.insert(key.into(), (&response).into());
                        if response.clicked() {
                            intents.push(Intent::View(view));
                        }
                    }
                });
        };
        let toggle_label = if rail { "Show labels" } else { "Hide labels" };
        if rail {
            nav(ui, &mut self.frame_widgets);
            let line = ui
                .allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover())
                .0;
            ui.painter().hline(
                line.shrink2(vec2(10.0, 0.0)).x_range(),
                line.center().y,
                Stroke::new(1.0, HAIRLINE()),
            );
            ui.add_space(6.0);
            // Short sidebars (terminal open) scroll the rail above the pinned labels toggle.
            ScrollArea::vertical()
                .id_salt("sidebar-rail-scroll")
                .auto_shrink([false, false])
                .max_height((ui.available_height() - 46.0).max(40.0))
                .show(ui, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        let branches = icon_button(
                            ui,
                            Icon::Branch,
                            true,
                            current == Some(View::Branches),
                            36.0,
                        )
                        .on_hover_text("Branches");
                        insert_visible(
                            &mut self.frame_widgets,
                            "view-branches",
                            &branches,
                            ui.clip_rect(),
                        );
                        Popup::menu(&branches).show(|ui| {
                            ui.label(RichText::new("BRANCHES").size(11.0).color(FAINT()));
                            if let Some(snapshot) = &tab.snapshot {
                                for branch in snapshot
                                    .branches
                                    .iter()
                                    .filter(|b| b.kind == BranchKind::Local)
                                {
                                    let label = short_ref(&branch.reference);
                                    if ui
                                        .button(if branch.is_head {
                                            format!("● {label}")
                                        } else {
                                            format!("   {label}")
                                        })
                                        .clicked()
                                        && let Ok(reference) =
                                            String::from_utf8(branch.reference.clone())
                                    {
                                        intents.push(Intent::History(Some(reference)));
                                        intents.push(Intent::View(View::Commits));
                                        ui.close();
                                    }
                                }
                            }
                            ui.separator();
                            if ui.button("Manage branches").clicked() {
                                intents.push(Intent::View(View::Branches));
                                ui.close();
                            }
                        });
                        for (image, label, key, selected, target) in [
                            (
                                Icon::Remote,
                                "Remotes",
                                "view-remotes",
                                current == Some(View::Remotes),
                                Target::View(View::Remotes),
                            ),
                            (
                                Icon::Stash,
                                "Stashes",
                                "view-stashes",
                                current == Some(View::Stashes),
                                Target::View(View::Stashes),
                            ),
                            (
                                Icon::Tag,
                                "Tags",
                                "view-tags",
                                page == Some(Page::Tags),
                                Target::Page(Page::Tags),
                            ),
                            (
                                Icon::Worktree,
                                "Worktrees",
                                "view-worktrees",
                                page == Some(Page::Worktrees),
                                Target::Page(Page::Worktrees),
                            ),
                            (
                                Icon::Modules,
                                "Submodules",
                                "view-submodules",
                                page == Some(Page::Submodules),
                                Target::Page(Page::Submodules),
                            ),
                            (
                                Icon::Database,
                                "Git LFS",
                                "view-lfs",
                                page == Some(Page::Lfs),
                                Target::Page(Page::Lfs),
                            ),
                            (
                                Icon::File,
                                "Path history",
                                "view-filehistory",
                                current == Some(View::FileHistory),
                                Target::View(View::FileHistory),
                            ),
                            (
                                Icon::Activity,
                                "Statistics",
                                "view-statistics",
                                page == Some(Page::Statistics),
                                Target::Page(Page::Statistics),
                            ),
                        ] {
                            let response =
                                icon_button(ui, image, true, selected, 36.0).on_hover_text(label);
                            response.widget_info(|| {
                                WidgetInfo::labeled(WidgetType::Button, true, label)
                            });
                            insert_visible(&mut self.frame_widgets, key, &response, ui.clip_rect());
                            if response.clicked() {
                                intents.push(target.intent());
                            }
                        }
                        let entries = self.integrations.entries().to_vec();
                        if !entries.is_empty() {
                            ui.add_space(4.0);
                            let (line, _) = ui.allocate_exact_size(vec2(28.0, 1.0), Sense::hover());
                            ui.painter().hline(line.x_range(), line.center().y, Stroke::new(1.0, HAIRLINE()));
                            ui.add_space(4.0);
                        }
                        for entry in entries {
                            let selected = page == Some(Page::Integrations)
                                && self.integrations.open_entry().is_some_and(|e| e.id == entry.id);
                            let response = letter_button(ui, &entry, selected).on_hover_text(&entry.name);
                            if selected && self.integrations.take_reveal() {
                                response.scroll_to_me(Some(Align::Center));
                            }
                            insert_visible(&mut self.frame_widgets, &format!("view-ext-{}", entry.id), &response, ui.clip_rect());
                            if !blocked {
                                insert_visible(&mut self.frame_widgets, &format!("int-package-{}", entry.id), &response, ui.clip_rect());
                            }
                            if response.clicked() {
                                intents.push(Intent::Extension(entry.id));
                            }
                        }
                    });
                });
        } else {
            nav(ui, &mut self.frame_widgets);
            Frame::new()
                .inner_margin(Margin {
                    left: 8,
                    right: 8,
                    top: 0,
                    bottom: 4,
                })
                .show(ui, |ui| {
                    let width = ui.available_width();
                    let r = search_field(ui, &mut tab.branch_filter, "Filter branches…", width);
                    self.frame_widgets.insert("sidebar-branch-filter".into(), (&r).into());
                });
            let filter = tab.branch_filter.to_lowercase();
            let footer = 46.0;
            ScrollArea::vertical()
                .id_salt("sidebar-scroll")
                .auto_shrink([false, false])
                .max_height((ui.available_height() - footer).max(60.0))
                .show(ui, |ui| {
                    Frame::new().inner_margin(Margin { left: 8, right: 8, top: 2, bottom: 10 }).show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 1.0;
                        let Some(snapshot) = &tab.snapshot else { return };
                        let mut local: Vec<_> = snapshot.branches.iter().filter(|b| b.kind == BranchKind::Local).collect();
                        local.sort_by_key(|b| !b.is_head);
                        let ahead_behind = snapshot.status.as_ref().and_then(|s| s.ahead_behind);
                        let header = section_header(ui, "Branches", Some(local.len()), "view-branches", current == Some(View::Branches), Some("New branch"), Some(&mut tab.worktree_view.sections[0]));
                        self.frame_widgets.insert("collapse-branches".into(), (&header.chevron).into());
                        self.frame_widgets.insert("view-branches".into(), (&header.open).into());
                        if header.open.clicked() { intents.push(Intent::View(View::Branches)); }
                        if header.add.as_ref().is_some_and(|a| a.clicked()) && writable { intents.push(Intent::Dialog(DialogKind::NewBranch)); }
                        if header.expanded {
                            for branch in local {
                                let label = short_ref(&branch.reference);
                                if !label.to_lowercase().contains(&filter) { continue; }
                                let meta = if branch.is_head { ahead_behind.map(|(a, b)| format!("↑{a} ↓{b}")) } else { None };
                                let response = branch_row(ui, &label, branch.is_head, meta.as_deref());
                                let response = crate::drag::branch(ui, response, tab.id, branch, intents, &mut self.frame_widgets);
                                let name = branch.reference.strip_prefix(b"refs/heads/").unwrap_or(&branch.reference).to_vec();
                                if response.clicked() && let Ok(reference) = String::from_utf8(branch.reference.clone()) {
                                    intents.push(Intent::History(Some(reference)));
                                    intents.push(Intent::View(View::Commits));
                                }
                                response.context_menu(|ui| {
                                    let mut context = crate::custom_commands::context(tab);
                                    context.branch = String::from_utf8(branch.reference.clone()).ok();
                                    crate::custom_commands::context_menu(ui, &self.commands, rocket_git::CommandScope::Branch, &context, intents, &mut self.frame_widgets);
                                    let target = crate::integrations::CommandTarget::branch(&tab.path, &branch.reference);
                                    crate::integrations::context_rows(ui, &target, crate::integrations::Divider::After, intents, &mut self.frame_widgets);
                                    if ui.add_enabled(writable && !branch.is_head, Button::new("Switch to branch")).clicked() {
                                        intents.push(Intent::Execute(tab.id, Action::SwitchBranch(name.clone())));
                                        ui.close();
                                    }
                                    if ui.button("Show history").clicked() && let Ok(reference) = String::from_utf8(branch.reference.clone()) {
                                        intents.push(Intent::History(Some(reference)));
                                        intents.push(Intent::View(View::Commits));
                                        ui.close();
                                    }
                                    ui.separator();
                                    if ui.add_enabled(writable && !branch.is_head, Button::new(format!("Merge into {}", tab.branch_label()))).clicked() {
                                        if let Ok(reference) = String::from_utf8(branch.reference.clone()) {
                                            intents.push(Intent::MergeBranch(reference));
                                        }
                                        ui.close();
                                    }
                                    if ui.add_enabled(writable && !branch.is_head, Button::new(format!("Rebase {} on top of {label}", tab.branch_label()))).clicked() {
                                        if let Ok(reference) = String::from_utf8(branch.reference.clone()) {
                                            intents.push(Intent::RebaseOnto(reference));
                                        }
                                        ui.close();
                                    }
                                    if ui.add_enabled(writable, Button::new("Rename")).clicked() {
                                        intents.push(Intent::Dialog(DialogKind::Rename(name.clone())));
                                        ui.close();
                                    }
                                    ui.separator();
                                    if ui.add_enabled(writable && !branch.is_head, Button::new(RichText::new("Delete branch").color(RED()))).clicked() {
                                        intents.push(Intent::Dialog(DialogKind::Confirm(
                                            Action::DeleteBranch(name.clone()),
                                            format!("Delete local branch {label}? Git will refuse unmerged branches."),
                                        )));
                                        ui.close();
                                    }
                                });
                                response.on_hover_text(format!("{label}\nClick to show its history · right-click for actions"));
                            }
                        }
                        ui.add_space(4.0);
                        let header = section_header(ui, "Remotes", Some(snapshot.remotes.len()), "view-remotes", current == Some(View::Remotes), Some("Add remote"), Some(&mut tab.worktree_view.sections[1]));
                        self.frame_widgets.insert("collapse-remotes".into(), (&header.chevron).into());
                        self.frame_widgets.insert("view-remotes".into(), (&header.open).into());
                        if header.open.clicked() { intents.push(Intent::View(View::Remotes)); }
                        if header.add.as_ref().is_some_and(|a| a.clicked()) && idle { intents.push(Intent::Dialog(DialogKind::AddRemote)); }
                        if header.expanded {
                            for remote in &snapshot.remotes {
                                let name = display(&remote.name);
                                if leaf_row(ui, &name, Icon::Remote, 0, None, false).clicked() {
                                    intents.push(Intent::View(View::Remotes));
                                }
                                for branch in snapshot.branches.iter().filter(|b| b.kind == BranchKind::Remote && b.symbolic_target.is_none()) {
                                    let label = short_ref(&branch.reference);
                                    let Some(short) = label.strip_prefix(&format!("{name}/")) else { continue };
                                    if !label.to_lowercase().contains(&filter) { continue; }
                                    let row = leaf_row(ui, short, Icon::Branch, 1, None, false);
                                    if crate::drag::branch(ui, row, tab.id, branch, intents, &mut self.frame_widgets).clicked()
                                        && let Ok(reference) = String::from_utf8(branch.reference.clone())
                                    {
                                        intents.push(Intent::History(Some(reference)));
                                        intents.push(Intent::View(View::Commits));
                                    }
                                }
                            }
                        }
                        ui.add_space(4.0);
                        if !tab.worktree_view.hide_stashes {
                        let header = section_header(ui, "Stashes", Some(snapshot.stashes.len()), "view-stashes", current == Some(View::Stashes), Some("Stash changes"), Some(&mut tab.worktree_view.sections[2]));
                        self.frame_widgets.insert("collapse-stashes".into(), (&header.chevron).into());
                        self.frame_widgets.insert("view-stashes".into(), (&header.open).into());
                        if header.open.clicked() { intents.push(Intent::View(View::Stashes)); }
                        if header.add.as_ref().is_some_and(|a| a.clicked()) && writable { intents.push(Intent::Dialog(DialogKind::Stash)); }
                        if header.expanded {
                            for stash in &snapshot.stashes {
                                if leaf_row(ui, &display(&stash.subject), Icon::Stash, 0, None, false).clicked() {
                                    intents.push(Intent::View(View::Stashes));
                                }
                            }
                        }
                        ui.add_space(4.0);
                        }
                        let header = section_header(ui, "Repository", None, "sidebar-repository", false, None, Some(&mut tab.worktree_view.sections[3]));
                        self.frame_widgets.insert("collapse-repository".into(), (&header.chevron).into());
                        if header.open.clicked() {
                            tab.worktree_view.sections[3] = !tab.worktree_view.sections[3];
                        }
                        if header.expanded {
                            for (image, label, key, selected, target) in [
                                (Icon::Tag, "Tags", "view-tags", page == Some(Page::Tags), Target::Page(Page::Tags)),
                                (Icon::Worktree, "Worktrees", "view-worktrees", page == Some(Page::Worktrees), Target::Page(Page::Worktrees)),
                                (Icon::Modules, "Submodules", "view-submodules", page == Some(Page::Submodules), Target::Page(Page::Submodules)),
                                (Icon::Database, "Git LFS", "view-lfs", page == Some(Page::Lfs), Target::Page(Page::Lfs)),
                                (Icon::File, "Path history", "view-filehistory", current == Some(View::FileHistory), Target::View(View::FileHistory)),
                                (Icon::Activity, "Statistics", "view-statistics", page == Some(Page::Statistics), Target::Page(Page::Statistics)),
                            ] {
                                if key == "view-tags" && tab.worktree_view.hide_tags { continue; }
                                let response = ui.add_enabled_ui(free || matches!(target, Target::View(_)), |ui| leaf_row(ui, label, image, 0, None, selected)).inner;
                                if selected {
                                    ui.painter().rect_filled(Rect::from_min_size(response.rect.left_top() + vec2(-8.0, 6.0), vec2(3.0, response.rect.height() - 12.0)), 1.5, BLUE());
                                }
                                let rect = response.rect.intersect(ui.clip_rect());
                                if rect.is_positive() {
                                    self.frame_widgets.insert(key.into(), UiControl { rect, enabled: response.enabled() });
                                }
                                if response.clicked() { intents.push(target.intent()); }
                            }
                        }
                        // One entry per enabled extension; with none on, no section at all.
                        let entries = self.integrations.entries().to_vec();
                        if !entries.is_empty() {
                            ui.add_space(4.0);
                            let header = section_header(ui, "Extensions", None, "sidebar-extensions", false, None, None);
                            self.frame_widgets.insert("collapse-extensions".into(), (&header.chevron).into());
                            if header.expanded {
                                for entry in entries {
                                    let selected = page == Some(Page::Integrations)
                                        && self.integrations.open_entry().is_some_and(|e| e.id == entry.id);
                                    let response = ui.add_enabled_ui(free, |ui| extension_row(ui, &entry, selected)).inner;
                                    if selected && self.integrations.take_reveal() {
                                        // Only as far as it takes: a clicked row stays under the pointer.
                                        response.scroll_to_me(None);
                                    }
                                    if selected {
                                        ui.painter().rect_filled(Rect::from_min_size(response.rect.left_top() + vec2(-8.0, 6.0), vec2(3.0, response.rect.height() - 12.0)), 1.5, BLUE());
                                    }
                                    insert_visible(&mut self.frame_widgets, &format!("view-ext-{}", entry.id), &response, ui.clip_rect());
                                    if !blocked {
                                        insert_visible(&mut self.frame_widgets, &format!("int-package-{}", entry.id), &response, ui.clip_rect());
                                    }
                                    if response.clicked() { intents.push(Intent::Extension(entry.id)); }
                                }
                            }
                        }
                    });
                });
        }
        // Labels toggle, pinned to the bottom of the sidebar.
        let bottom = ui.max_rect().bottom();
        let rect = Rect::from_min_max(
            pos2(ui.max_rect().left(), bottom - 44.0),
            pos2(ui.max_rect().right(), bottom),
        );
        ui.painter()
            .hline(rect.x_range(), rect.top(), Stroke::new(1.0, HAIRLINE()));
        let row = Rect::from_min_max(rect.min + vec2(8.0, 7.0), rect.max - vec2(8.0, 7.0));
        let response = ui.interact(row, Id::new("sidebar-labels"), Sense::click());
        if response.hovered() {
            ui.painter().rect_filled(row, 7.0, HOVER());
        }
        let color = if response.hovered() { TEXT() } else { FAINT() };
        let icon_x = if rail {
            row.center().x
        } else {
            row.left() + 18.0
        };
        icon(
            ui.painter(),
            Rect::from_center_size(pos2(icon_x, row.center().y), vec2(15.0, 15.0)),
            Icon::Layout,
            color,
        );
        if !rail {
            ui.painter().text(
                pos2(row.left() + 36.0, row.center().y),
                Align2::LEFT_CENTER,
                toggle_label,
                FontId::proportional(12.5),
                color,
            );
        }
        response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, toggle_label));
        self.frame_widgets
            .insert("sidebar-labels".into(), (&response).into());
        let clicked = response.clicked();
        response.on_hover_text(if rail {
            "Show sidebar labels"
        } else {
            "Hide labels and show icons only"
        });
        if clicked {
            let ctx = ui.ctx().clone();
            self.toggle_sidebar_labels(&ctx);
        }
    }
}

enum Target {
    View(View),
    Page(Page),
}
impl Target {
    fn intent(&self) -> Intent {
        match self {
            Target::View(view) => Intent::View(*view),
            Target::Page(page) => Intent::Dialog(page.dialog_kind()),
        }
    }
}

/// Full-width status line above a view, such as a read error or warning.
fn banner(ui: &mut Ui, color: Color32, text: &str) {
    Frame::new()
        .fill(color.gamma_multiply(0.1))
        .inner_margin(Margin::symmetric(14, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.add(Label::new(RichText::new(text).color(color)).wrap());
        });
}

/// What a paused banner recovers. A lone cherry-pick or revert has nothing to skip to:
/// Skip would do what Abort does, so it isn't offered. `skip` is whether it can be used.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Recover {
    Rebase,
    Import,
    Commit { operation: CommitOperation, skip: Option<bool> },
}

impl Recover {
    /// What this state pauses, if it's one of the states with this banner.
    pub(crate) fn of(state: &OperationState, sequence: Option<&CommitSequence>) -> Option<Self> {
        if state.bisect {
            None
        } else if state.mail_import {
            Some(Self::Import)
        } else if state.rebase {
            Some(Self::Rebase)
        } else if !state.merge_heads.is_empty() {
            None
        } else if !state.sequencer && state.cherry_pick != state.revert {
            let operation = if state.cherry_pick { CommitOperation::CherryPick } else { CommitOperation::Revert };
            Some(Self::Commit { operation, skip: None })
        } else if let Some(CommitSequence { operation, remaining: Some(_), .. }) = sequence
            && state.sequencer
            && sequence_markers_match(state, *operation)
        {
            // Only a sequence Rocket reviewed, still matching Git's sequencer, gets
            // recovery controls; each review checks both again.
            Some(Self::Commit { operation: *operation, skip: Some(state.cherry_pick || state.revert) })
        } else {
            None
        }
    }
    pub(crate) fn intent(self, way: CommitRecovery) -> Intent {
        use CommitRecovery::*;
        Intent::Dialog(match self {
            Self::Import => DialogKind::MailImport(match way {
                Continue => MailImportAction::Continue,
                Skip => MailImportAction::Skip,
                Abort => MailImportAction::Abort,
            }),
            Self::Rebase => DialogKind::Rebase(match way {
                Continue => RebaseAction::Continue,
                Skip => RebaseAction::Skip,
                Abort => RebaseAction::Abort,
            }),
            Self::Commit { operation, .. } => DialogKind::CommitAction(CommitActionRequest {
                operation,
                targets: Vec::new(),
                mainline: None,
                recovery: Some(way),
            }),
        })
    }
    /// The banner's name for what's paused, and for one step of it.
    fn names(self) -> (&'static str, &'static str) {
        match self {
            Self::Import => ("Import", "patch"),
            Self::Rebase => ("Rebase", "commit"),
            Self::Commit { operation: CommitOperation::CherryPick, .. } => ("Cherry-pick", "commit"),
            Self::Commit { .. } => ("Revert", "commit"),
        }
    }
    fn abort_label(self) -> &'static str {
        match self {
            Self::Import => "Abort import",
            Self::Rebase => "Abort rebase",
            Self::Commit { operation: CommitOperation::CherryPick, .. } => "Abort cherry-pick",
            Self::Commit { .. } => "Abort revert",
        }
    }
}

/// What a paused rebase or commit import says, and whether it can go on as it stands.
pub(crate) struct Pause {
    pub title: String,
    pub detail: String,
    pub ready: bool,
}

pub(crate) fn pause_text(state: &OperationState, status: Option<&Status>, recover: Recover) -> Pause {
    let (what, noun) = recover.names();
    let lone = matches!(recover, Recover::Commit { skip: None, .. });
    let title = match state.step.as_ref().filter(|_| !lone) {
        Some(step) => format!("{what} paused at {noun} {} of {}", step.at, step.total),
        None => format!("{what} paused"),
    };
    let kind = state.step.as_ref().map_or(StepKind::Commit, |s| s.kind);
    let tracked: Vec<&StatusEntry> = status
        .map(|s| s.entries.iter().filter(|e| !matches!(e.kind, EntryKind::Untracked | EntryKind::Ignored)).collect())
        .unwrap_or_default();
    let conflicts: Vec<_> = tracked.iter().filter(|e| e.kind == EntryKind::Unmerged).collect();
    let unstaged = tracked.iter().any(|e| e.worktree != Change::Unchanged);
    let staged = tracked.iter().any(|e| e.index != Change::Unchanged);
    let between = matches!(recover, Recover::Commit { .. }) && kind == StepKind::Between;
    // Only what the review will accept may promise a Continue.
    let moved = ("the branch has moved, so finish this rebase with Git.".to_owned(), false);
    let (next, ready) = match conflicts.as_slice() {
        _ if kind == StepKind::BranchMoved => moved,
        _ if status.is_none() => ("check Changes, then Continue.".to_owned(), false),
        [one] => {
            let path = one.path.display_lossy();
            let name = path.rsplit('/').next().unwrap_or_default().to_owned();
            (format!("fix the conflict in {name}, stage it, then Continue."), false)
        }
        [_, ..] => (format!("fix the {} conflicts, stage them, then Continue.", conflicts.len()), false),
        [] if between && (staged || unstaged) => ("between commits, with changes that aren't committed. Commit or stash them, or Abort.".to_owned(), false),
        [] if unstaged => ("stage your changes, then Continue.".to_owned(), false),
        [] => match kind {
            StepKind::Message => ("your changes are in, so Continue saves the message.".to_owned(), true),
            StepKind::Between if between => ("between commits. Continue does the rest, or Abort ends it.".to_owned(), true),
            StepKind::Between if staged => ("stopped as planned. Unstage your changes, or Skip to discard them.".to_owned(), false),
            StepKind::BranchMoved => moved,
            StepKind::Between => ("stopped as planned. Continue when you're ready.".to_owned(), true),
            StepKind::Edit if staged => ("stopped as planned. Continue adds what's staged to this commit.".to_owned(), true),
            StepKind::Edit => ("stopped as planned. Change it, or Continue to keep it.".to_owned(), true),
            StepKind::Commit if staged => ("your fix is staged, so you can Continue.".to_owned(), true),
            StepKind::Commit if lone => ("no changes are left here. Stage a fix, or Abort.".to_owned(), false),
            // The patch may be in already, or Git may have failed to apply it at all.
            StepKind::Commit if recover == Recover::Import => ("nothing is staged for this patch. Stage what it should change, or Skip it.".to_owned(), false),
            StepKind::Commit => (format!("no changes are left here. Stage a fix, or Skip this {noun}."), false),
        },
    };
    let detail = match state.step.as_ref().filter(|s| !s.subject.is_empty()) {
        Some(step) => format!("{} · {next}", step.subject),
        None => {
            let mut letters = next.chars();
            letters.next().map_or_else(String::new, |c| c.to_uppercase().chain(letters).collect())
        }
    };
    Pause { title, detail, ready }
}

#[cfg(test)]
mod pause_tests {
    use super::{Recover, pause_text};
    use rocket_git::*;

    fn entry(path: &str, kind: EntryKind, index: Change, worktree: Change) -> StatusEntry {
        StatusEntry {
            path: RepoPath::new(path.as_bytes().to_vec()).unwrap(),
            original_path: None,
            kind,
            index,
            worktree,
            submodule: None,
            similarity: None,
        }
    }

    #[test]
    fn the_banner_says_where_it_stopped_and_what_comes_next() {
        let step = OperationStep { at: 2, total: 4, kind: StepKind::Commit, subject: "Add a lexer".into(), branch: None };
        let state = OperationState { mail_import: true, step: Some(step), ..Default::default() };
        let mut status = Status {
            head: Head::Unborn { branch: b"main".to_vec() },
            upstream: None,
            ahead_behind: None,
            entries: vec![entry("src/lexer.rs", EntryKind::Unmerged, Change::Unmerged, Change::Unmerged)],
        };
        let pause = pause_text(&state, Some(&status), Recover::Import);
        assert_eq!(pause.title, "Import paused at patch 2 of 4");
        assert_eq!(pause.detail, "Add a lexer · fix the conflict in lexer.rs, stage it, then Continue.");
        assert!(!pause.ready);
        status.entries.push(entry("b.rs", EntryKind::Unmerged, Change::Unmerged, Change::Unmerged));
        assert!(pause_text(&state, Some(&status), Recover::Import).detail.ends_with("fix the 2 conflicts, stage them, then Continue."));
        status.entries.clear();
        status.entries.push(entry("src/lexer.rs", EntryKind::Tracked, Change::Modified, Change::Modified));
        let pause = pause_text(&state, Some(&status), Recover::Import);
        assert!(pause.detail.ends_with("stage your changes, then Continue.") && !pause.ready);
        status.entries.clear();
        status.entries.push(entry("src/lexer.rs", EntryKind::Tracked, Change::Modified, Change::Unchanged));
        let pause = pause_text(&state, Some(&status), Recover::Import);
        assert!(pause.detail.ends_with("your fix is staged, so you can Continue.") && pause.ready);
        status.entries.clear();
        let pause = pause_text(&state, Some(&status), Recover::Import);
        assert!(pause.detail.ends_with("nothing is staged for this patch. Stage what it should change, or Skip it.") && !pause.ready, "git am --continue refuses this");
        let rebase = OperationState { rebase: true, ..Default::default() };
        let pause = pause_text(&rebase, None, Recover::Rebase);
        assert_eq!(pause.title, "Rebase paused");
        assert_eq!(pause.detail, "Check Changes, then Continue.");
        assert!(!pause.ready, "nothing is known yet");
    }

    /// Each kind of stop promises only what the rebase review accepts.
    #[test]
    fn a_rebase_stop_promises_only_what_continue_accepts() {
        let at = |kind| OperationState {
            rebase: true,
            step: Some(OperationStep { at: 2, total: 3, kind, subject: String::new(), branch: None }),
            ..Default::default()
        };
        let mut status = Status { head: Head::Unborn { branch: b"main".to_vec() }, upstream: None, ahead_behind: None, entries: Vec::new() };
        let clean = |kind| pause_text(&at(kind), Some(&status), Recover::Rebase);
        assert!(!clean(StepKind::Commit).ready, "an empty commit is refused with empty_change");
        assert!(clean(StepKind::Commit).detail.ends_with("Skip this commit."));
        assert!(clean(StepKind::Edit).ready && clean(StepKind::Between).ready && clean(StepKind::Message).ready);
        const GIT: &str = "finish this rebase with Git.";
        let moved = clean(StepKind::BranchMoved);
        assert!(!moved.ready && moved.detail.ends_with(GIT), "every way on is refused: {}", moved.detail);
        status.entries.push(entry("a.rs", EntryKind::Tracked, Change::Modified, Change::Unchanged));
        let staged = |kind| pause_text(&at(kind), Some(&status), Recover::Rebase);
        assert!(staged(StepKind::Commit).ready && staged(StepKind::Edit).ready);
        assert!(!staged(StepKind::Between).ready, "staged changes at a break are refused");
        assert!(!staged(StepKind::BranchMoved).ready);
        status.entries.push(entry("b.rs", EntryKind::Unmerged, Change::Unmerged, Change::Unmerged));
        let conflict = pause_text(&at(StepKind::BranchMoved), Some(&status), Recover::Rebase);
        assert!(conflict.detail.ends_with(GIT), "fixing the conflict wouldn't help: {}", conflict.detail);
    }

    /// A lone commit has no count and nothing to skip to; a range has both.
    #[test]
    fn a_stopped_revert_says_the_same_things_as_a_rebase() {
        let status = |entries| Status { head: Head::Unborn { branch: b"main".to_vec() }, upstream: None, ahead_behind: None, entries };
        let conflict = || vec![entry("src/lexer.rs", EntryKind::Unmerged, Change::Unmerged, Change::Unmerged)];
        let stopped = |at, total, kind, subject: &str| OperationState {
            revert: true,
            step: Some(OperationStep { at, total, kind, subject: subject.into(), branch: None }),
            ..Default::default()
        };
        let lone = Recover::Commit { operation: CommitOperation::Revert, skip: None };
        let range = Recover::Commit { operation: CommitOperation::Revert, skip: Some(true) };
        let pause = pause_text(&stopped(1, 1, StepKind::Commit, "Add a lexer"), Some(&status(conflict())), lone);
        assert_eq!(pause.title, "Revert paused");
        assert_eq!(pause.detail, "Add a lexer · fix the conflict in lexer.rs, stage it, then Continue.");
        assert!(!pause.ready);
        let pause = pause_text(&stopped(2, 3, StepKind::Commit, "Add a lexer"), Some(&status(conflict())), range);
        assert_eq!(pause.title, "Revert paused at commit 2 of 3");
        let clean = stopped(1, 1, StepKind::Commit, "Add a lexer");
        assert!(pause_text(&clean, Some(&status(Vec::new())), lone).detail.ends_with("Stage a fix, or Abort."));
        let clean = stopped(2, 3, StepKind::Commit, "Add a lexer");
        assert!(pause_text(&clean, Some(&status(Vec::new())), range).detail.ends_with("Skip this commit."));
        let between = OperationState { sequencer: true, step: Some(OperationStep { at: 2, total: 3, kind: StepKind::Between, subject: String::new(), branch: None }), ..Default::default() };
        let pick = Recover::Commit { operation: CommitOperation::CherryPick, skip: Some(false) };
        let pause = pause_text(&between, Some(&status(Vec::new())), pick);
        assert_eq!(pause.title, "Cherry-pick paused at commit 2 of 3");
        assert_eq!(pause.detail, "Between commits. Continue does the rest, or Abort ends it.");
        assert!(pause.ready);
        let dirty = vec![entry("a.rs", EntryKind::Tracked, Change::Modified, Change::Unchanged)];
        let pause = pause_text(&between, Some(&status(dirty)), pick);
        assert!(!pause.ready, "the review refuses this with dirty_worktree");
        assert!(pause.detail.contains("Commit or stash them"), "{}", pause.detail);
    }
}

/// The dots of a short series: done, where it stopped, still to come.
fn step_dots(ui: &mut Ui, step: &OperationStep) {
    const SPACE: f32 = 14.0;
    let (rect, _) = ui.allocate_exact_size(vec2(SPACE * step.total as f32, 28.0), Sense::hover());
    for i in 0..step.total {
        let center = pos2(rect.left() + SPACE * (i as f32 + 0.5), rect.center().y);
        match (i + 1).cmp(&step.at) {
            std::cmp::Ordering::Less => ui.painter().circle_filled(center, 4.5, GREEN()),
            std::cmp::Ordering::Equal => ui.painter().circle_filled(center, 4.5, AMBER()),
            std::cmp::Ordering::Greater => ui.painter().circle_stroke(center, 3.8, Stroke::new(1.5, RING_IDLE())),
        };
    }
}

/// A paused rebase or commit import: where it stopped, what to do, and the ways on.
fn paused_banner(
    ui: &mut Ui,
    state: &OperationState,
    status: Option<&Status>,
    recover: Recover,
    enabled: bool,
    intents: &mut Vec<Intent>,
    widgets: &mut std::collections::HashMap<String, UiControl>,
) {
    let pause = pause_text(state, status, recover);
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(vec2(28.0, 28.0), Sense::hover());
        ui.painter().rect_filled(rect, 7.0, VIOLET().gamma_multiply(0.22));
        icon(ui.painter(), rect.shrink(7.0), Icon::Branch, VIOLET());
        ui.add_space(4.0);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            let abort = ui.add_enabled(enabled, Button::new(recover.abort_label()));
            crate::pages::control(widgets, "operation-abort", &abort, ui.clip_rect());
            if abort.clicked() {
                intents.push(recover.intent(CommitRecovery::Abort));
            }
            let can_skip = match recover {
                Recover::Commit { skip, .. } => skip,
                _ => Some(true),
            };
            if let Some(can_skip) = can_skip {
                let skip = ui.add_enabled(enabled && can_skip, Button::new(if recover == Recover::Import { "Skip patch" } else { "Skip commit" }));
                crate::pages::control(widgets, "operation-skip", &skip, ui.clip_rect());
                if skip.clicked() {
                    intents.push(recover.intent(CommitRecovery::Skip));
                }
            }
            let next = if pause.ready {
                crate::pages::primary(ui, enabled, "Continue")
            } else {
                ui.add_enabled(enabled, Button::new("Continue"))
            };
            crate::pages::control(widgets, "operation-continue", &next, ui.clip_rect());
            if next.clicked() {
                intents.push(recover.intent(CommitRecovery::Continue));
            }
            if let Some(step) = state.step.as_ref().filter(|s| (2..=12).contains(&s.total)) {
                step_dots(ui, step);
            }
            ui.with_layout(Layout::top_down(Align::Min), |ui| {
                ui.spacing_mut().item_spacing.y = 1.0;
                ui.add(Label::new(RichText::new(&pause.title).font(semibold(13.0))).truncate());
                ui.add(Label::new(RichText::new(&pause.detail).size(12.5).color(MUTED())).truncate());
            });
        });
    });
}

/// Merge, rebase, cherry-pick or revert in progress: one banner with its recovery actions.
fn operation_banner(
    ui: &mut Ui,
    state: &OperationState,
    status: Option<&Status>,
    sequence: Option<&CommitSequence>,
    enabled: bool,
    intents: &mut Vec<Intent>,
    widgets: &mut std::collections::HashMap<String, UiControl>,
) {
    Frame::new()
        .fill(VIOLET().gamma_multiply(0.12))
        .inner_margin(Margin::symmetric(14, 9))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            if let Some(recover) = Recover::of(state, sequence) {
                paused_banner(ui, state, status, recover, enabled, intents, widgets);
                return;
            }
            ui.horizontal_wrapped(|ui| {
                let (rect, _) = ui.allocate_exact_size(vec2(28.0, 28.0), Sense::hover());
                ui.painter().rect_filled(rect, 7.0, VIOLET().gamma_multiply(0.22));
                icon(ui.painter(), rect.shrink(7.0), Icon::Branch, VIOLET());
                ui.add_space(4.0);
                if state.bisect {
                    ui.label(RichText::new("Bisect in progress").font(semibold(13.0)));
                    ui.label(RichText::new("Test the checkout, mark the result, or restore your original checkout.").color(MUTED()));
                    let r = ui.add_enabled(enabled, Button::new("Open bisect"));
                    crate::pages::control(widgets, "operation-bisect", &r, ui.clip_rect());
                    if r.clicked() { intents.push(Intent::Dialog(DialogKind::Bisect)); }
                } else if state.merge_committed {
                    ui.label(RichText::new("Merge commit exists").font(semibold(13.0)));
                    ui.label(RichText::new("Clear leftover state before another operation. Your commit and files stay in place.").color(MUTED()));
                    let finish = ui.add_enabled(enabled, Button::new("Clear merge state"));
                    crate::pages::control(widgets, "operation-finish-merge", &finish, ui.clip_rect());
                    if finish.clicked() {
                        intents.push(Intent::Dialog(DialogKind::Confirm(Action::FinishMerge, "HEAD already contains this merge commit. Clear Git's leftover merge state?".into())));
                    }
                } else if !state.merge_heads.is_empty() {
                    ui.label(RichText::new("Merge in progress").font(semibold(13.0)));
                    ui.label(RichText::new("Resolve conflicts, stage resolutions, then continue from Changes.").color(MUTED()));
                    let abort = ui.add_enabled(enabled, Button::new("Abort merge"));
                    crate::pages::control(widgets, "operation-abort", &abort, ui.clip_rect());
                    if abort.clicked() {
                        intents.push(Intent::Dialog(DialogKind::Confirm(Action::AbortMerge, "Abort the current merge and discard its conflict resolutions?".into())));
                    }
                } else if state.sequencer && (state.cherry_pick || state.revert || sequence.is_some()) {
                    ui.label(RichText::new("A cherry-pick or revert sequence is in progress that does not match a Rocket review. Finish it with Git before starting new operations.").color(AMBER()));
                } else {
                    ui.label(RichText::new("Another Git operation is in progress. Finish it in Git before starting new operations.").color(AMBER()));
                }
            });
        });
}

/// A stopped commit's marker, if any, belongs to the reviewed sequence's operation.
fn sequence_markers_match(state: &OperationState, operation: CommitOperation) -> bool {
    match (state.cherry_pick, state.revert) {
        (false, false) => true,
        (true, false) => operation == CommitOperation::CherryPick,
        (false, true) => operation == CommitOperation::Revert,
        (true, true) => false,
    }
}

fn small_icon(ui: &mut Ui, image: Icon, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(vec2(13.0, 13.0), Sense::hover());
    icon(ui.painter(), rect, image, color);
}

fn divider(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(9.0, 22.0), Sense::hover());
    ui.painter()
        .vline(rect.center().x, rect.y_range(), Stroke::new(1.0, HAIRLINE()));
}

/// Square icon-only button with hover and selected states.
pub(crate) fn icon_button(
    ui: &mut Ui,
    image: Icon,
    enabled: bool,
    selected: bool,
    size: f32,
) -> Response {
    let response = ui.add_enabled(
        enabled,
        Button::new("").frame(false).min_size(vec2(size, size)),
    );
    let rect = response.rect;
    if selected {
        ui.painter().rect_filled(rect, 7.0, ACCENT_SOFT());
    } else if response.hovered() || response.has_focus() {
        ui.painter().rect_filled(rect, 7.0, HOVER());
    }
    let color = if !enabled {
        FAINT().gamma_multiply(0.6)
    } else if selected {
        BLUE()
    } else if response.hovered() {
        TEXT()
    } else {
        MUTED()
    };
    icon(
        ui.painter(),
        Rect::from_center_size(rect.center(), vec2(16.0, 16.0)),
        image,
        color,
    );
    response
}

#[derive(Default)]
struct ToolOptions {
    enabled: bool,
    label: bool,
    count: u64,
    busy: bool,
    chevron: bool,
}

/// Toolbar action: icon, optional label, optional count pill and dropdown chevron.
fn tool_button(ui: &mut Ui, image: Icon, label: &str, options: ToolOptions) -> Response {
    let font = FontId::proportional(13.0);
    let text_width = if options.label {
        ui.painter()
            .layout_no_wrap(label.into(), font.clone(), TEXT())
            .size()
            .x
            + 7.0
    } else {
        0.0
    };
    let count = (options.count > 0).then(|| options.count.to_string());
    let count_width = count.as_ref().map_or(0.0, |c| c.len() as f32 * 7.0 + 18.0);
    let width =
        10.0 + 16.0 + text_width + count_width + if options.chevron { 14.0 } else { 0.0 } + 10.0;
    let response = ui.add_enabled(
        options.enabled,
        Button::new("").frame(false).min_size(vec2(width, 32.0)),
    );
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, options.enabled, label));
    let rect = response.rect;
    let painter = ui.painter();
    if options.enabled
        && (response.hovered()
            || response.has_focus()
            || Popup::is_id_open(ui.ctx(), Popup::default_response_id(&response)))
    {
        painter.rect_filled(rect, 7.0, HOVER());
    }
    let (icon_color, text_color) = if options.enabled {
        (MUTED(), TEXT())
    } else {
        (FAINT().gamma_multiply(0.6), FAINT())
    };
    let icon_rect =
        Rect::from_center_size(pos2(rect.left() + 18.0, rect.center().y), vec2(16.0, 16.0));
    if options.busy {
        let t = ui.input(|i| i.time) as f32 * 5.0;
        let points: Vec<Pos2> = (0..=12)
            .map(|i| {
                let a = t + i as f32 * 0.38;
                icon_rect.center() + vec2(a.cos(), a.sin()) * 6.5
            })
            .collect();
        painter.add(Shape::line(points, Stroke::new(1.8, BLUE())));
        ui.ctx().request_repaint();
    } else {
        icon(painter, icon_rect, image, icon_color);
    }
    let mut x = icon_rect.right() + 7.0;
    if options.label {
        painter.text(
            pos2(x, rect.center().y),
            Align2::LEFT_CENTER,
            label,
            font,
            text_color,
        );
        x += text_width;
    }
    if let Some(count) = count {
        let pill = count_pill(
            painter,
            pos2(x + count_width / 2.0 - 3.0, rect.center().y),
            &count,
            if options.enabled { BLUE() } else { FAINT() },
        );
        x = pill.right() + 4.0;
    }
    if options.chevron {
        icon(
            painter,
            Rect::from_center_size(pos2(x + 5.0, rect.center().y), vec2(10.0, 10.0)),
            Icon::Chevron,
            FAINT(),
        );
    }
    response
}

/// The toolbar search field; it opens the command palette rather than filtering in place.
fn command_field(ui: &mut Ui, compact: bool) -> Response {
    let binding = crate::shortcuts::hint(ui.ctx(), "Ctrl+K");
    let tooltip = if binding.is_empty() { "Search or run a command".into() } else { format!("Search or run a command ({binding})") };
    if compact {
        return icon_button(ui, Icon::Search, true, false, 32.0)
            .on_hover_text(tooltip);
    }
    let response = ui
        .add(Button::new("").frame(false).min_size(vec2(290.0, 32.0)))
        .on_hover_text(tooltip);
    response
        .widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, "Search or run a command"));
    let rect = response.rect;
    let painter = ui.painter();
    painter.rect(
        rect,
        7.0,
        BG(),
        Stroke::new(
            1.0,
            if response.hovered() {
                EDGE_HOT()
            } else {
                BORDER()
            },
        ),
        StrokeKind::Inside,
    );
    icon(
        painter,
        Rect::from_center_size(pos2(rect.left() + 17.0, rect.center().y), vec2(14.0, 14.0)),
        Icon::Search,
        FAINT(),
    );
    painter.text(
        pos2(rect.left() + 32.0, rect.center().y),
        Align2::LEFT_CENTER,
        "Search commands, branches…",
        FontId::proportional(12.5),
        FAINT(),
    );
    let key_text = painter.layout_no_wrap(binding.clone(), FontId::proportional(11.0), MUTED());
    // Long pairs of bindings remain in the tooltip, without overlapping search text.
    if !binding.is_empty() && key_text.size().x <= 65.0 {
        let width = key_text.size().x + 12.0;
        let key = Rect::from_center_size(pos2(rect.right() - width * 0.5 - 7.0, rect.center().y), vec2(width, 20.0));
        painter.rect(key, 4.0, BG(), Stroke::new(1.0, BORDER()), StrokeKind::Inside);
        painter.galley(key.center() - key_text.size() * 0.5, key_text, MUTED());
    }
    response
}

/// Small rounded status-bar chip.
fn chip(ui: &mut Ui, image: Icon, label: &str, selected: bool, tint: Option<Color32>) -> Response {
    let font = FontId::proportional(12.0);
    let width = ui
        .painter()
        .layout_no_wrap(label.into(), font.clone(), TEXT())
        .size()
        .x
        + 32.0;
    let response = ui.add(Button::new("").frame(false).min_size(vec2(width, 20.0)));
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, label));
    let rect = response.rect;
    let color = tint.unwrap_or(if selected { BLUE() } else { MUTED() });
    if tint.is_some() || selected {
        ui.painter().rect_filled(
            rect,
            10.0,
            color.gamma_multiply(if response.hovered() { 0.24 } else { 0.14 }),
        );
    } else if response.hovered() {
        ui.painter().rect_filled(rect, 10.0, HOVER());
    }
    icon(
        ui.painter(),
        Rect::from_center_size(pos2(rect.left() + 13.0, rect.center().y), vec2(11.0, 11.0)),
        image,
        color,
    );
    ui.painter().text(
        pos2(rect.left() + 23.0, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        font,
        if tint.is_some() || selected {
            color
        } else {
            MUTED()
        },
    );
    response
}

/// One open repository in the Repositories section: its folder name and branch, a
/// dot for working changes and, on hover or focus, a close button.
fn repository_row(ui: &mut Ui, tab: &Tab, selected: bool) -> (Response, Response) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 32.0), Sense::hover());
    let close_rect =
        Rect::from_center_size(pos2(rect.right() - 14.0, rect.center().y), vec2(22.0, 22.0));
    let row = ui.interact(
        Rect::from_min_max(rect.min, pos2(close_rect.left(), rect.bottom())),
        Id::new(("repository-row", tab.id)),
        Sense::click(),
    );
    let close = ui.interact(
        close_rect,
        Id::new(("repository-row-close", tab.id)),
        Sense::click(),
    );
    let active = row.hovered() || close.hovered() || row.has_focus() || close.has_focus();
    style::row_background(ui.painter(), rect, selected, active);
    if selected {
        ui.painter().rect_filled(
            Rect::from_min_size(
                pos2(rect.left() - 8.0, rect.top() + 8.0),
                vec2(3.0, rect.height() - 16.0),
            ),
            1.5,
            BLUE(),
        );
    }
    let dirty = tab
        .snapshot
        .as_ref()
        .and_then(|s| s.status.as_ref())
        .is_some_and(|s| !s.entries.is_empty());
    icon(
        ui.painter(),
        Rect::from_center_size(pos2(rect.left() + 18.0, rect.center().y), vec2(15.0, 15.0)),
        Icon::Folder,
        if selected { BLUE() } else { MUTED() },
    );
    if dirty {
        ui.painter().circle(
            pos2(rect.left() + 25.0, rect.center().y - 6.0),
            3.5,
            AMBER(),
            Stroke::new(2.0, PANEL()),
        );
    }
    let font = if selected {
        semibold(13.0)
    } else {
        FontId::proportional(13.0)
    };
    let right = if active {
        close_rect.left() - 4.0
    } else {
        rect.right() - 6.0
    };
    let name_width = ui
        .painter()
        .layout_no_wrap(tab.title.clone(), font.clone(), TEXT())
        .size()
        .x;
    let name_end = (rect.left() + 36.0 + name_width).min(right);
    ui.painter()
        .with_clip_rect(Rect::from_min_max(
            pos2(rect.left() + 36.0, rect.top()),
            pos2(name_end, rect.bottom()),
        ))
        .text(
            pos2(rect.left() + 36.0, rect.center().y),
            Align2::LEFT_CENTER,
            &tab.title,
            font,
            if selected || active { TEXT() } else { MUTED() },
        );
    let branch = tab.branch_label();
    if name_end + 30.0 < right && !branch.is_empty() {
        clipped_text(
            ui,
            Rect::from_min_max(pos2(name_end + 8.0, rect.top()), pos2(right, rect.bottom())),
            &branch,
            12.0,
            FAINT(),
        );
    }
    if active {
        if close.hovered() || close.has_focus() {
            ui.painter()
                .rect_filled(close_rect.shrink(1.0), 4.0, RAISED());
        }
        let c = close_rect.center();
        for delta in [-3.5, 3.5] {
            ui.painter().line_segment(
                [c + vec2(-3.5, delta), c + vec2(3.5, -delta)],
                Stroke::new(1.2, if close.hovered() { TEXT() } else { FAINT() }),
            );
        }
    }
    let title = tab.title.clone();
    row.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, &title));
    close.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, format!("Close {title}")));
    (
        row.on_hover_text(crate::views::display_path(&tab.path)),
        close.on_hover_text("Close repository"),
    )
}

fn nav_row(
    ui: &mut Ui,
    label: &str,
    image: Icon,
    selected: bool,
    enabled: bool,
    count: Option<usize>,
    rail: bool,
) -> Response {
    let response = ui.add_enabled(
        enabled,
        Button::new("")
            .frame(false)
            .min_size(vec2(ui.available_width(), 32.0)),
    );
    let r = response.rect;
    style::row_background(
        ui.painter(),
        r,
        selected,
        response.hovered() || response.has_focus(),
    );
    if selected {
        ui.painter().rect_filled(
            Rect::from_min_size(
                pos2(r.left() - 8.0, r.top() + 8.0),
                vec2(3.0, r.height() - 16.0),
            ),
            1.5,
            BLUE(),
        );
    }
    let color = if !enabled {
        FAINT()
    } else if selected || response.hovered() {
        TEXT()
    } else {
        MUTED()
    };
    let icon_x = if rail { r.center().x } else { r.left() + 18.0 };
    icon(
        ui.painter(),
        Rect::from_center_size(pos2(icon_x, r.center().y), vec2(15.0, 15.0)),
        image,
        if selected { BLUE() } else { color },
    );
    if rail {
        if count.is_some() {
            ui.painter().circle(
                pos2(r.right() - 9.0, r.top() + 8.0),
                3.5,
                BLUE(),
                Stroke::new(2.0, PANEL()),
            );
        }
    } else {
        ui.painter().text(
            pos2(r.left() + 36.0, r.center().y),
            Align2::LEFT_CENTER,
            label,
            if selected {
                semibold(13.0)
            } else {
                FontId::proportional(13.0)
            },
            color,
        );
        if let Some(count) = count {
            count_pill(
                ui.painter(),
                pos2(r.right() - 20.0, r.center().y),
                &count.to_string(),
                BLUE(),
            );
        }
    }
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, enabled, label));
    if rail {
        response.on_hover_text(match count {
            Some(n) => format!("{label} · {n}"),
            None => label.into(),
        })
    } else {
        response
    }
}

struct SectionHeader {
    chevron: Response,
    open: Response,
    add: Option<Response>,
    expanded: bool,
}

/// Sidebar section: the chevron collapses it, the title opens its management view and
/// the trailing + starts the section's create action.
fn section_header(
    ui: &mut Ui,
    label: &str,
    count: Option<usize>,
    _key: &str,
    selected: bool,
    add: Option<&str>,
    state: Option<&mut bool>,
) -> SectionHeader {
    let id = Id::new(("sidebar-section", label));
    let mut expanded = state.as_deref().copied().unwrap_or_else(|| ui.ctx().data_mut(|d| *d.get_persisted_mut_or(id, true)));
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::hover());
    let chevron_rect = Rect::from_min_size(rect.min, vec2(20.0, rect.height()));
    let chevron = ui.interact(chevron_rect, id.with("chevron"), Sense::click());
    let add_rect =
        Rect::from_center_size(pos2(rect.right() - 12.0, rect.center().y), vec2(22.0, 22.0));
    let title_rect = Rect::from_min_max(
        pos2(chevron_rect.right(), rect.top()),
        pos2(
            if add.is_some() {
                add_rect.left() - 2.0
            } else {
                rect.right()
            },
            rect.bottom(),
        ),
    );
    let open = ui.interact(title_rect, id.with("open"), Sense::click());
    let add = add.map(|hint| {
        ui.interact(add_rect, id.with("add"), Sense::click())
            .on_hover_text(hint)
    });
    if chevron.clicked() {
        expanded = !expanded;
        if let Some(state) = state { *state = expanded; }
        else { ui.ctx().data_mut(|d| d.insert_persisted(id, expanded)); }
    }
    let hovered = open.hovered() || chevron.hovered() || add.as_ref().is_some_and(|a| a.hovered());
    if selected {
        ui.painter().rect_filled(rect, 6.0, SELECTED());
    } else if open.hovered() {
        ui.painter().rect_filled(rect, 6.0, HOVER());
    }
    let chevron_icon = Rect::from_center_size(chevron_rect.center(), vec2(9.0, 9.0));
    icon(
        ui.painter(),
        chevron_icon,
        if expanded {
            Icon::Chevron
        } else {
            Icon::ChevronRight
        },
        if chevron.hovered() { TEXT() } else { FAINT() },
    );
    let text = label.to_uppercase();
    let galley = ui.painter().layout_no_wrap(
        text,
        semibold(11.0),
        if hovered || selected { MUTED() } else { FAINT() },
    );
    let text_width = galley.size().x;
    ui.painter().galley(
        pos2(
            title_rect.left() + 2.0,
            rect.center().y - galley.size().y / 2.0,
        ),
        galley,
        FAINT(),
    );
    if let Some(count) = count {
        ui.painter().text(
            pos2(title_rect.left() + text_width + 9.0, rect.center().y),
            Align2::LEFT_CENTER,
            count.to_string(),
            FontId::proportional(11.5),
            FAINT(),
        );
    }
    if let Some(add) = &add
        && hovered
    {
        if add.hovered() {
            ui.painter().rect_filled(add_rect, 5.0, RAISED());
        }
        icon(
            ui.painter(),
            add_rect.shrink(6.0),
            Icon::Plus,
            if add.hovered() { TEXT() } else { MUTED() },
        );
    }
    open.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, label));
    chevron.widget_info(|| {
        WidgetInfo::labeled(
            WidgetType::Button,
            true,
            format!("{} {label}", if expanded { "Collapse" } else { "Expand" }),
        )
    });
    SectionHeader {
        chevron,
        open,
        add,
        expanded,
    }
}

fn branch_row(ui: &mut Ui, label: &str, current: bool, meta: Option<&str>) -> Response {
    let response = ui.add(
        Button::new("")
            .frame(false)
            .min_size(vec2(ui.available_width(), 28.0)),
    );
    let rect = response.rect;
    style::row_background(
        ui.painter(),
        rect,
        false,
        response.hovered() || response.has_focus(),
    );
    let dot = pos2(rect.left() + 26.0, rect.center().y);
    if current {
        ui.painter().circle_filled(dot, 4.0, BLUE());
    } else {
        ui.painter()
            .circle_stroke(dot, 3.5, Stroke::new(1.6, FAINT()));
    }
    let meta_width = meta.map_or(0.0, |m| {
        ui.painter()
            .layout_no_wrap(m.into(), FontId::proportional(11.5), FAINT())
            .size()
            .x
            + 10.0
    });
    let color = if current || response.hovered() {
        TEXT()
    } else {
        MUTED()
    };
    let text = Rect::from_min_max(
        pos2(rect.left() + 38.0, rect.top()),
        pos2(rect.right() - 6.0 - meta_width, rect.bottom()),
    );
    let mut job = text::LayoutJob::simple_singleline(
        label.into(),
        if current {
            semibold(13.0)
        } else {
            FontId::proportional(13.0)
        },
        color,
    );
    job.wrap = text::TextWrapping::truncate_at_width(text.width().max(0.0));
    let galley = ui.painter().layout_job(job);
    ui.painter().galley(
        pos2(text.left(), rect.center().y - galley.size().y / 2.0),
        galley,
        color,
    );
    if let Some(meta) = meta {
        ui.painter().text(
            pos2(rect.right() - 6.0, rect.center().y),
            Align2::RIGHT_CENTER,
            meta,
            FontId::proportional(11.5),
            BLUE(),
        );
    }
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, label));
    response
}

fn leaf_row(
    ui: &mut Ui,
    label: &str,
    image: Icon,
    depth: u8,
    meta: Option<&str>,
    selected: bool,
) -> Response {
    let response = ui.add(
        Button::new("")
            .frame(false)
            .min_size(vec2(ui.available_width(), 28.0)),
    );
    let rect = response.rect;
    let enabled = response.enabled();
    style::row_background(
        ui.painter(),
        rect,
        selected,
        enabled && (response.hovered() || response.has_focus()),
    );
    let indent = 20.0 + depth as f32 * 16.0;
    icon(
        ui.painter(),
        Rect::from_center_size(
            pos2(rect.left() + indent + 6.0, rect.center().y),
            vec2(13.0, 13.0),
        ),
        image,
        FAINT(),
    );
    let color = if !enabled {
        FAINT()
    } else if response.hovered() {
        TEXT()
    } else {
        MUTED()
    };
    let meta_width = meta.map_or(0.0, |m| {
        ui.painter()
            .layout_no_wrap(m.into(), FontId::proportional(11.5), FAINT())
            .size()
            .x
            + 10.0
    });
    clipped_text(
        ui,
        Rect::from_min_max(
            pos2(rect.left() + indent + 20.0, rect.top()),
            pos2(rect.right() - 6.0 - meta_width, rect.bottom()),
        ),
        label,
        13.0,
        color,
    );
    if let Some(meta) = meta {
        ui.painter().text(
            pos2(rect.right() - 6.0, rect.center().y),
            Align2::RIGHT_CENTER,
            meta,
            FontId::proportional(11.5),
            FAINT(),
        );
    }
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, enabled, label));
    response
}

/// A sidebar entry for an enabled extension: its letter tile, its name, and an amber dot
/// when it waits for a review or for a check of an uncertain write.
fn extension_row(ui: &mut Ui, entry: &crate::integrations::Entry, selected: bool) -> Response {
    let response = ui.add(
        Button::new("")
            .frame(false)
            .min_size(vec2(ui.available_width(), 28.0)),
    );
    let rect = response.rect;
    let enabled = response.enabled();
    style::row_background(
        ui.painter(),
        rect,
        selected,
        enabled && (response.hovered() || response.has_focus()),
    );
    crate::integrations::letter_tile(
        ui.painter(),
        Rect::from_center_size(pos2(rect.left() + 26.0, rect.center().y), vec2(16.0, 16.0)),
        entry.letter,
        if selected { BLUE() } else { MUTED() },
    );
    let color = if !enabled {
        FAINT()
    } else if response.hovered() {
        TEXT()
    } else {
        MUTED()
    };
    let right = if entry.attention { 20.0 } else { 6.0 };
    clipped_text(
        ui,
        Rect::from_min_max(pos2(rect.left() + 40.0, rect.top()), pos2(rect.right() - right, rect.bottom())),
        &entry.name,
        13.0,
        color,
    );
    if entry.attention {
        ui.painter().circle_filled(pos2(rect.right() - 12.0, rect.center().y), 3.5, AMBER());
    }
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, enabled, &entry.name));
    response
}

/// An enabled extension in the icon rail.
fn letter_button(ui: &mut Ui, entry: &crate::integrations::Entry, selected: bool) -> Response {
    let response = ui.add(Button::new("").frame(false).min_size(vec2(36.0, 36.0)));
    let rect = response.rect;
    if selected {
        ui.painter().rect_filled(rect, 7.0, ACCENT_SOFT());
    } else if response.hovered() || response.has_focus() {
        ui.painter().rect_filled(rect, 7.0, HOVER());
    }
    crate::integrations::letter_tile(
        ui.painter(),
        Rect::from_center_size(rect.center(), vec2(20.0, 20.0)),
        entry.letter,
        if selected { BLUE() } else { MUTED() },
    );
    if entry.attention {
        ui.painter().circle_filled(rect.right_top() + vec2(-8.0, 8.0), 3.5, AMBER());
    }
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, &entry.name));
    response
}

/// Publish only the visible part of a scrolled control, so automation never clicks
/// a control hidden under the pinned sidebar footer.
fn insert_visible(
    widgets: &mut std::collections::HashMap<String, UiControl>,
    key: &str,
    response: &Response,
    clip: Rect,
) {
    let rect = response.rect.intersect(clip);
    if rect.is_positive() {
        widgets.insert(
            key.into(),
            UiControl {
                rect,
                enabled: response.enabled(),
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        app::{Rocket, View},
        test_common as common,
    };
    use eframe::egui::{Context, Event, Modifiers, PointerButton, Pos2, RawInput, Rect, vec2};
    use std::{
        thread,
        time::{Duration, Instant},
    };

    #[test]
    fn background_refresh_keeps_sidebar_pages_enabled() {
        common::isolated(
            "ROCKET_CHROME_UI_CASE",
            "chrome::tests::background_refresh_keeps_sidebar_pages_enabled",
            || {
                let fixture = common::Fixture::new();
                fixture.write("file", "first");
                fixture.commit("first");
                let ctx = Context::default();
                let mut app = Rocket::with_log_directory(
                    &ctx,
                    vec![fixture.repository.clone()],
                    fixture.root.join("logs"),
                );
                let deadline = Instant::now() + Duration::from_secs(40);
                while app.busy() {
                    app.poll(&ctx);
                    assert!(Instant::now() < deadline);
                    thread::sleep(Duration::from_millis(10));
                }
                app.auto_refresh = false;
                app.tabs[0].view = View::Commits;
                // A watcher-driven refresh that stays running while the frame is drawn.
                app.tabs[0].refresh = Some(
                    rocket_git::ReadTask::spawn(|cancel| {
                        while !cancel.is_cancelled() {
                            thread::sleep(Duration::from_millis(5));
                        }
                        Err(rocket_git::Error::Cancelled)
                    })
                    .unwrap(),
                );
                app.tabs[0].background_refresh = true;
                assert!(app.busy() && !app.interaction_busy());
                for _ in 0..2 {
                    app.frame_widgets.clear();
                    let mut output = ctx.run_ui(
                        RawInput {
                            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1480.0, 920.0))),
                            ..Default::default()
                        },
                        |ui| app.shell(ui, &mut Vec::new()),
                    );
                    output.textures_delta.clear();
                }
                for key in ["view-tags", "view-worktrees", "view-submodules", "view-lfs"] {
                    assert!(
                        app.frame_widgets[key].enabled,
                        "{key} was disabled by a background refresh"
                    );
                }
                app.tabs[0].refresh.take().unwrap().cancel();
            },
        );
    }

    fn draw(ctx: &Context, app: &mut Rocket) {
        app.frame_widgets.clear();
        let mut output = ctx.run_ui(
            RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1480.0, 920.0))),
                ..Default::default()
            },
            |ui| app.shell(ui, &mut Vec::new()),
        );
        output.textures_delta.clear();
    }

    /// One frame of the shell with input events, like `draw`.
    fn draw_with(ctx: &Context, app: &mut Rocket, events: Vec<Event>) {
        app.frame_widgets.clear();
        let mut output = ctx.run_ui(
            RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1480.0, 920.0))),
                events,
                ..Default::default()
            },
            |ui| app.shell(ui, &mut Vec::new()),
        );
        output.textures_delta.clear();
    }

    /// Press and release the primary button on a published control.
    fn click(ctx: &Context, app: &mut Rocket, id: &str) {
        let at = app.frame_widgets[id].rect.center();
        let button = |pressed| Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        draw_with(ctx, app, vec![Event::PointerMoved(at), button(true)]);
        draw_with(ctx, app, vec![button(false)]);
    }

    #[test]
    fn drawer_details_open_on_request_and_git_output_stays_folded() {
        common::isolated(
            "ROCKET_CHROME_UI_CASE",
            "chrome::tests::drawer_details_open_on_request_and_git_output_stays_folded",
            || {
                let fixture = common::Fixture::new();
                let ctx = Context::default();
                let mut app = Rocket::with_log_directory(&ctx, vec![], fixture.root.join("logs"));
                let outcome = crate::backend::Outcome {
                    summary: "Rejected: the remote has newer commits".into(),
                    details: vec!["Nothing changed on the remote.".into()],
                    raw_output: "! [rejected] main -> main (fetch first)".into(),
                    ..Default::default()
                };
                app.activities.push(crate::app::Activity {
                    title: "Push".into(),
                    outcome,
                    elapsed: Duration::from_millis(2100),
                    at: std::time::SystemTime::now(),
                    seen: false,
                });
                app.show_console = true;
                draw(&ctx, &mut app);
                for id in ["activity-open", "console-clear", "console-close", "console-details-0"] {
                    assert!(app.frame_widgets.contains_key(id), "the drawer shows {id}");
                }
                assert!(
                    !app.frame_widgets.contains_key("console-output-0"),
                    "details start folded"
                );
                click(&ctx, &mut app, "console-details-0");
                draw(&ctx, &mut app);
                assert!(app.frame_widgets.contains_key("console-copy-0"));
                assert!(
                    app.frame_widgets.contains_key("console-output-0"),
                    "the Git output is offered"
                );
                click(&ctx, &mut app, "console-clear");
                draw(&ctx, &mut app);
                assert!(app.activities.is_empty());
                assert!(!app.frame_widgets.contains_key("console-details-0"));
            },
        );
    }

    #[test]
    fn the_drawer_collapses_to_its_strip_and_opens_again_at_its_height() {
        common::isolated(
            "ROCKET_CHROME_UI_CASE",
            "chrome::tests::the_drawer_collapses_to_its_strip_and_opens_again_at_its_height",
            || {
                let fixture = common::Fixture::new();
                let ctx = Context::default();
                let mut app = Rocket::with_log_directory(&ctx, vec![], fixture.root.join("logs"));
                app.activities.push(crate::app::Activity {
                    title: "Push".into(),
                    outcome: crate::backend::Outcome {
                        summary: "Rejected: the remote has newer commits".into(),
                        details: vec!["Nothing changed on the remote.".into()],
                        ..Default::default()
                    },
                    elapsed: Duration::from_millis(1700),
                    at: std::time::SystemTime::now(),
                    seen: false,
                });
                app.show_console = true;
                draw(&ctx, &mut app);
                assert!(app.frame_widgets.contains_key("console-details-0"));
                let open = app.frame_widgets["drawer-collapse"].rect;
                assert_eq!(app.frame_widgets["console-close"].rect, open, "the old id still works");
                assert!(!app.frame_widgets.contains_key("drawer-summary"));

                click(&ctx, &mut app, "drawer-collapse");
                draw(&ctx, &mut app);
                assert!(app.show_console && app.drawer_collapsed, "collapsed, not hidden");
                for id in ["console-details-0", "activity-open", "console-clear", "drawer-collapse"] {
                    assert!(!app.frame_widgets.contains_key(id), "the strip has no {id}");
                }
                // The 36 px strip sits on the 26 px status bar of the 920 px window.
                let strip = app.frame_widgets["drawer-summary"].rect;
                assert!(strip.top() >= 920.0 - 26.0 - 36.0 && strip.bottom() <= 920.0 - 26.0, "{strip:?}");
                assert!(app.frame_widgets.contains_key("drawer-expand"));

                // The one-line summary opens the drawer again at the height it had.
                click(&ctx, &mut app, "drawer-summary");
                draw(&ctx, &mut app);
                assert!(!app.drawer_collapsed);
                assert_eq!(app.frame_widgets["drawer-collapse"].rect, open);
                assert!(app.frame_widgets.contains_key("console-details-0"));

                // The toolbar button, Ctrl+` and the palette open a collapsed drawer,
                // then hide it completely. (No repository here, so no toolbar.)
                click(&ctx, &mut app, "drawer-collapse");
                app.toggle_activity();
                assert!(app.show_console && !app.drawer_collapsed, "the first toggle opens it");
                draw(&ctx, &mut app);
                app.toggle_activity();
                draw(&ctx, &mut app);
                assert!(!app.show_console);
                assert!(!app.frame_widgets.contains_key("drawer-expand"));
                assert!(!app.frame_widgets.contains_key("drawer-summary"));

                // A tab opens a collapsed drawer on its pane.
                app.show_console = true;
                app.drawer_collapsed = true;
                draw(&ctx, &mut app);
                click(&ctx, &mut app, "drawer-terminal");
                assert!(app.show_terminal && !app.show_console && !app.drawer_collapsed);
            },
        );
    }

    #[test]
    fn the_terminal_menu_acts_on_the_shell_not_on_activity_or_git() {
        common::isolated(
            "ROCKET_CHROME_UI_CASE",
            "chrome::tests::the_terminal_menu_acts_on_the_shell_not_on_activity_or_git",
            || {
                let fixture = common::Fixture::new();
                let ctx = Context::default();
                let mut app = Rocket::with_log_directory(&ctx, vec![], fixture.root.join("logs"));
                draw(&ctx, &mut app);
                click(&ctx, &mut app, "menu-terminal");
                draw(&ctx, &mut app);
                for id in ["terminal-new", "terminal-console"] {
                    assert!(app.frame_widgets.contains_key(id), "the menu has {id}");
                }
                // Without a shell its actions are there but disabled.
                for id in ["terminal-interrupt-menu", "terminal-clear-scrollback-menu", "terminal-end-menu"] {
                    assert!(!app.frame_widgets[id].enabled, "{id} needs a session");
                }
                // Clear Output emptied Activity and Stop Process cancelled the Git
                // operation; the other two were never enabled.
                for id in ["terminal-clear", "terminal-stop", "terminal-external", "terminal-shell"] {
                    assert!(!app.frame_widgets.contains_key(id), "{id} is gone");
                }
            },
        );
    }

    #[test]
    fn status_light_skips_short_work_and_bridges_gaps() {
        let mut light = super::BusyIndicator::default();
        let read = Some("Refreshing");
        // Work shorter than the show delay never shows the light.
        assert_eq!(light.update(read, 0.0), (false, Some(0.25)));
        assert!(!light.update(read, 0.2).0);
        assert!(!light.update(None, 0.21).0);
        // Longer work shows it, and a short gap before the next read keeps it on.
        assert!(!light.update(read, 1.0).0);
        assert!(light.update(read, 1.3).0);
        assert!(light.update(None, 1.31).0);
        assert!(light.update(read, 1.32).0);
        // It goes once work has stopped for the hold time.
        assert!(light.update(None, 1.6).0);
        assert_eq!(light.update(None, 1.8), (false, None));
    }

    #[test]
    fn a_settings_save_waiting_out_its_debounce_is_not_status_work() {
        common::isolated(
            "ROCKET_CHROME_UI_CASE",
            "chrome::tests::a_settings_save_waiting_out_its_debounce_is_not_status_work",
            || {
                let fixture = common::Fixture::new();
                let ctx = Context::default();
                let mut app = Rocket::with_log_directory(&ctx, vec![], fixture.root.join("logs"));
                let deadline = Instant::now() + Duration::from_secs(40);
                while app.busy() {
                    app.poll(&ctx);
                    assert!(Instant::now() < deadline, "startup settles");
                    thread::sleep(Duration::from_millis(10));
                }
                app.show_details = !app.show_details;
                app.poll(&ctx);
                assert!(app.state_pending(), "the changed setting waits to be saved");
                assert!(app.busy(), "busy() still covers the pending save");
                assert!(!app.working(), "the status bar does not show it as work");
            },
        );
    }

    #[test]
    fn retry_save_is_reachable_at_supported_widths_and_resumes_saving() {
        common::isolated(
            "ROCKET_CHROME_UI_CASE",
            "chrome::tests::retry_save_is_reachable_at_supported_widths_and_resumes_saving",
            || {
                let fixture = common::Fixture::new();
                let ctx = Context::default();
                let mut app = Rocket::with_storage(&ctx, vec![], fixture.root.join("logs"), fixture.root.join("state"));
                let deadline = Instant::now() + Duration::from_secs(40);
                while app.busy() {
                    app.poll(&ctx);
                    assert!(Instant::now() < deadline);
                    thread::sleep(Duration::from_millis(10));
                }
                app.state_error = Some("The previous settings write failed.".into());
                app.show_details = !app.show_details;
                for width in [780.0, 1480.0] {
                    let bounds = Rect::from_min_size(Pos2::ZERO, vec2(width, 920.0));
                    for _ in 0..2 {
                        app.frame_widgets.clear();
                        ctx.run_ui(RawInput { screen_rect: Some(bounds), ..Default::default() }, |ui| app.shell(ui, &mut Vec::new()))
                            .drop_without_applying_deltas();
                    }
                    let retry = &app.frame_widgets["status-retry-save"];
                    assert!(retry.enabled);
                    assert!(bounds.contains_rect(retry.rect), "retry control must fit at {width}");
                }
                click(&ctx, &mut app, "status-retry-save");
                assert!(app.state_error.is_none());
                app.poll(&ctx);
                while app.busy() {
                    app.poll(&ctx);
                    assert!(Instant::now() < deadline);
                    thread::sleep(Duration::from_millis(10));
                }
                assert!(app.state_error.is_none());
                draw(&ctx, &mut app);
                assert!(!app.frame_widgets.contains_key("status-retry-save"));
            },
        );
    }

    #[test]
    fn showing_activity_clears_the_review_markers() {
        common::isolated(
            "ROCKET_CHROME_UI_CASE",
            "chrome::tests::showing_activity_clears_the_review_markers",
            || {
                let fixture = common::Fixture::new();
                let ctx = Context::default();
                let mut app = Rocket::with_log_directory(&ctx, vec![], fixture.root.join("logs"));
                app.activities.push(crate::app::Activity {
                    title: "Push".into(),
                    outcome: Default::default(), // A failed command.
                    elapsed: Duration::ZERO,
                    at: std::time::SystemTime::now(),
                    seen: false,
                });
                app.show_console = false;
                draw(&ctx, &mut app);
                assert!(app.frame_widgets.contains_key("status-review"));
                app.show_console = true;
                draw(&ctx, &mut app);
                assert!(
                    !app.frame_widgets.contains_key("status-review"),
                    "the open drawer already shows the failure"
                );
                app.show_console = false;
                draw(&ctx, &mut app);
                assert!(
                    !app.frame_widgets.contains_key("status-review"),
                    "seen stays seen"
                );
            },
        );
    }

    #[test]
    fn a_diff_reload_keeps_the_shown_diff_instead_of_a_spinner() {
        common::isolated(
            "ROCKET_CHROME_UI_CASE",
            "chrome::tests::a_diff_reload_keeps_the_shown_diff_instead_of_a_spinner",
            || {
                let fixture = common::Fixture::new();
                fixture.write("file", "first\n");
                fixture.commit("first");
                let ctx = Context::default();
                let mut app = Rocket::with_log_directory(
                    &ctx,
                    vec![fixture.repository.clone()],
                    fixture.root.join("logs"),
                );
                let deadline = Instant::now() + Duration::from_secs(40);
                while app.busy() || app.tabs.first().is_none_or(|t| t.patch.is_none()) {
                    app.poll(&ctx);
                    assert!(
                        Instant::now() < deadline,
                        "the selected commit's diff loads"
                    );
                    thread::sleep(Duration::from_millis(10));
                }
                app.auto_refresh = false;
                app.show_details = true;
                app.tabs[0].view = View::Commits;
                draw(&ctx, &mut app);
                assert!(app.frame_widgets.contains_key("diff-syntax"));
                // The same selection reloads, as after a background refresh.
                app.tabs[0].diff = Some(
                    rocket_git::ReadTask::spawn(|cancel| {
                        while !cancel.is_cancelled() {
                            thread::sleep(Duration::from_millis(5));
                        }
                        Err(rocket_git::Error::Cancelled)
                    })
                    .unwrap(),
                );
                draw(&ctx, &mut app);
                assert!(
                    app.frame_widgets.contains_key("diff-syntax"),
                    "the diff stays"
                );
                assert!(
                    !app.frame_widgets.contains_key("diff-cancel"),
                    "no spinner over it"
                );
                // A new selection has nothing to show yet, so it gets the spinner.
                app.tabs[0].patch = None;
                draw(&ctx, &mut app);
                assert!(app.frame_widgets.contains_key("diff-cancel"));
                app.tabs[0].diff.take().unwrap().cancel();
            },
        );
    }

    mod indicators {
        use crate::{
            app::{ActiveWrite, DialogKind, Rocket},
            batch::flow_tests::{click, frame, open, wait},
            test_common as common,
        };
        use eframe::egui::{Context, Event, Modifiers, PointerButton};
        use std::{
            sync::mpsc,
            thread,
            time::{Duration, Instant},
        };

        /// A write that runs until it is stopped, as if Git were still working.
        fn run_until_stopped(app: &mut Rocket, label: &str) {
            let (_, fetch_progress) = mpsc::sync_channel(1);
            let (_, clone_progress) = mpsc::sync_channel(1);
            let (_, force_push_progress) = mpsc::sync_channel(1);
            let task = rocket_git::WriteTask::spawn(|cancel| {
                while !cancel.is_cancelled() {
                    thread::sleep(Duration::from_millis(5));
                }
                Err(rocket_git::Error::Cancelled)
            })
            .unwrap();
            app.write = Some(ActiveWrite {
                owner: app.tab().map(|t| t.id),
                label: label.into(),
                title: crate::operation_names::running(label),
                task,
                fetch_progress,
                clone_progress,
                force_push_progress,
                force_push_phase: None,
                progress: "Checking out the next revision".into(),
                fraction: None,
                cancelling: false,
                started: Instant::now(),
                submitted_draft: None,
            });
        }

        #[test]
        fn first_stop_click_survives_progress_changes() {
            common::isolated(
                "ROCKET_CHROME_UI_CASE",
                "chrome::tests::indicators::first_stop_click_survives_progress_changes",
                || {
                    let fixture = common::Fixture::new();
                    fixture.write("file", "x");
                    fixture.commit("first");
                    let ctx = Context::default();
                    let mut app = open(&ctx, &fixture);
                    let mut missed = Vec::new();
                    for collapsed in [true, false] {
                        for (before, after) in [(None, Some(0.4)), (Some(0.4), None), (None, None)] {
                            run_until_stopped(&mut app, "push");
                            app.show_console = true;
                            app.drawer_collapsed = collapsed;
                            let write = app.write.as_mut().unwrap();
                            write.progress = "Starting".into();
                            write.fraction = before;
                            frame(&ctx, &mut app, vec![]);
                            frame(&ctx, &mut app, vec![]);
                            let target = if collapsed { "status-cancel" } else { "cancel-operation" };
                            let pos = app.frame_widgets[target].rect.center();
                            // Progress can arrive after input hit-testing but before UI layout.
                            let write = app.write.as_mut().unwrap();
                            write.progress = "Sending objects and checking the remote result".into();
                            write.fraction = after;
                            let press = |pressed| Event::PointerButton {
                                pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE,
                            };
                            frame(&ctx, &mut app, vec![Event::PointerMoved(pos), press(true), press(false)]);
                            let accepted = app.write.as_ref().is_some_and(|w| w.cancelling);
                            if !accepted {
                                missed.push(format!("{target}: {before:?} -> {after:?}"));
                            }
                            // Always release the fixture worker, including on the failing baseline.
                            let mut write = app.write.take().unwrap();
                            write.task.cancel();
                            let deadline = Instant::now() + Duration::from_secs(10);
                            while write.task.try_finish().is_none() {
                                assert!(Instant::now() < deadline, "fixture write did not stop");
                                thread::sleep(Duration::from_millis(5));
                            }
                        }
                    }
                    assert!(missed.is_empty(), "first Stop click missed: {missed:?}");
                },
            );
        }

        #[test]
        fn a_running_operation_has_one_stop_where_it_is_shown_most_closely() {
            common::isolated(
                "ROCKET_CHROME_UI_CASE",
                "chrome::tests::indicators::a_running_operation_has_one_stop_where_it_is_shown_most_closely",
                || {
                    let fixture = common::Fixture::new();
                    fixture.write("file", "x");
                    fixture.commit("first");
                    let ctx = Context::default();
                    let mut app = open(&ctx, &fixture);
                    run_until_stopped(&mut app, "bisect-good");
                    let shown = |app: &Rocket, id: &str| app.frame_widgets.contains_key(id);
                    // Nothing else shows it: the status bar has the spinner and Stop.
                    app.show_console = true;
                    app.drawer_collapsed = true;
                    frame(&ctx, &mut app, vec![]);
                    assert!(shown(&app, "status-cancel"), "the status bar has Stop");
                    assert!(!shown(&app, "cancel-operation"));
                    // The open Activity drawer has it; the status bar only names it.
                    app.drawer_collapsed = false;
                    frame(&ctx, &mut app, vec![]);
                    assert!(shown(&app, "cancel-operation"), "the Activity row has Stop");
                    assert!(!shown(&app, "status-cancel"));
                    // Its dialog is open: the status bar and Activity only name it...
                    app.show_dialog(DialogKind::Bisect);
                    let state = app.dialog.as_mut().unwrap().bisect.as_mut().unwrap();
                    state.task = None;
                    state.applying = true;
                    super::draw(&ctx, &mut app);
                    assert!(!shown(&app, "status-cancel") && !shown(&app, "cancel-operation"));
                    // ...and the footer has only Stop beside the status line.
                    frame(&ctx, &mut app, vec![]);
                    frame(&ctx, &mut app, vec![]);
                    assert!(shown(&app, "cancel-operation"), "the footer has Stop");
                    assert!(!shown(&app, "status-cancel"));
                    for id in ["dialog-submit", "dialog-preview-refresh"] {
                        assert!(!shown(&app, id), "{id} waits until the operation ends");
                    }
                    // No footer Close either: its old id is the header's ×.
                    assert_eq!(app.frame_widgets["dialog-cancel"].rect, app.frame_widgets["dialog-close"].rect);
                    click(&ctx, &mut app, "cancel-operation");
                    assert!(app.write.as_ref().is_some_and(|w| w.cancelling), "Stop stops it");
                    wait(&ctx, &mut app);
                    assert!(app.write.is_none());
                },
            );
        }
    }

    mod repositories {
        use crate::{
            app::{Intent, Rocket},
            batch::flow_tests::{click, enabled, frame, wait},
            test_common as common,
        };
        use eframe::egui::{
            Context, Event, Key, Modifiers, PointerButton, Pos2, RawInput, Rect, vec2,
        };

        const CASE: &str = "ROCKET_CHROME_UI_CASE";

        fn two_repositories() -> (common::Fixture, common::Fixture, Context, Rocket) {
            let (a, b) = (common::Fixture::new(), common::Fixture::new());
            for f in [&a, &b] {
                f.write("file", "x");
                f.commit("first");
            }
            let ctx = Context::default();
            let mut app = Rocket::with_log_directory(
                &ctx,
                vec![a.repository.clone(), b.repository.clone()],
                a.root.join("logs"),
            );
            wait(&ctx, &mut app);
            app.auto_refresh = false;
            // Record folders instead of opening File Explorer.
            app.opened_urls = Some(Vec::new());
            (a, b, ctx, app)
        }
        fn right_click(ctx: &Context, app: &mut Rocket, key: &str) {
            assert!(enabled(ctx, app, key), "{key} is disabled");
            let pos = app.frame_widgets[key].rect.center();
            let press = |pressed| Event::PointerButton {
                pos,
                button: PointerButton::Secondary,
                pressed,
                modifiers: Modifiers::NONE,
            };
            frame(
                ctx,
                app,
                vec![Event::PointerMoved(pos), press(true), press(false)],
            );
        }
        /// One frame with a key press, through the app's shortcuts as in `update`.
        fn shortcut(ctx: &Context, app: &mut Rocket, modifiers: Modifiers, key: Key) {
            let mut output = ctx.run_ui(
                RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1480.0, 1000.0))),
                    events: vec![Event::Key {
                        key,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers,
                    }],
                    ..Default::default()
                },
                |ui| {
                    let mut intents = Vec::new();
                    app.shortcuts(ui.ctx(), &mut intents);
                    for intent in intents {
                        app.dispatch(intent);
                    }
                },
            );
            output.textures_delta.clear();
        }

        #[test]
        fn the_tab_menu_shows_the_folder_and_closed_repositories_reopen_newest_first() {
            common::isolated(
                CASE,
                "chrome::tests::repositories::the_tab_menu_shows_the_folder_and_closed_repositories_reopen_newest_first",
                || {
                    let (a, b, ctx, mut app) = two_repositories();
                    let (first, second) = (app.tabs[0].id, app.tabs[1].id);
                    right_click(&ctx, &mut app, &format!("tab-{first}"));
                    click(&ctx, &mut app, "tab-menu-explorer");
                    let shown = app.opened_urls.clone().unwrap();
                    assert_eq!(shown.len(), 1, "{shown:?}");
                    assert!(
                        shown[0].starts_with("explorer:")
                            && !shown[0].starts_with(r"explorer:\\?\")
                            && shown[0].ends_with("repo with spaces 文"),
                        "{shown:?}"
                    );
                    app.dispatch(Intent::CloseTab(first));
                    app.dispatch(Intent::CloseTab(second));
                    assert!(app.tabs.is_empty());
                    // Ctrl+Shift+T brings back the last closed one first.
                    shortcut(&ctx, &mut app, Modifiers::CTRL | Modifiers::SHIFT, Key::T);
                    wait(&ctx, &mut app);
                    // Tabs keep the canonical folder they opened.
                    let canonical = |f: &common::Fixture| f.repository.canonicalize().unwrap();
                    assert_eq!(app.tabs.len(), 1);
                    assert_eq!(app.tabs[0].path, canonical(&b));
                    // The + menu reopens the next one.
                    click(&ctx, &mut app, "tab-add");
                    click(&ctx, &mut app, "tab-reopen");
                    wait(&ctx, &mut app);
                    let paths: Vec<_> = app.tabs.iter().map(|t| t.path.clone()).collect();
                    assert_eq!(paths, [canonical(&b), canonical(&a)]);
                    assert!(app.closed_tabs.is_empty());
                    app.reopen_closed_tab();
                    assert_eq!(
                        app.notice.as_deref(),
                        Some("No closed repository to reopen.")
                    );
                },
            );
        }

        #[test]
        fn the_repositories_list_replaces_the_tab_strip_while_the_full_sidebar_shows() {
            common::isolated(
                CASE,
                "chrome::tests::repositories::the_repositories_list_replaces_the_tab_strip_while_the_full_sidebar_shows",
                || {
                    let (_a, _b, ctx, mut app) = two_repositories();
                    let (first, second) = (app.tabs[0].id, app.tabs[1].id);
                    app.repository_list = true;
                    frame(&ctx, &mut app, vec![]);
                    assert!(!app.frame_widgets.contains_key(&format!("tab-{first}")));
                    assert!(
                        app.frame_widgets
                            .contains_key(&format!("repository-row-{second}"))
                    );
                    let other = if app.active == 0 { second } else { first };
                    click(&ctx, &mut app, &format!("repository-row-{other}"));
                    assert_eq!(app.tab().unwrap().id, other);
                    // The row has the tab's menu.
                    right_click(&ctx, &mut app, &format!("repository-row-{first}"));
                    assert!(enabled(&ctx, &mut app, "tab-menu-explorer"));
                    // Without the full sidebar the tabs come back.
                    app.show_sidebar = false;
                    frame(&ctx, &mut app, vec![]);
                    assert!(app.frame_widgets.contains_key(&format!("tab-{first}")));
                    app.show_sidebar = true;
                    click(&ctx, &mut app, &format!("repository-close-{first}"));
                    assert_eq!(app.tabs.len(), 1);
                    assert_eq!(app.closed_tabs.len(), 1);
                },
            );
        }
    }
}
