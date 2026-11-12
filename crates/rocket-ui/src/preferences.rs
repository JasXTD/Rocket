//! Preferences: one dialog with a page list. General, Security, Appearance and Commit messages
//! apply at once; External tools and MCP server keep their own Save, as before. Keyboard
//! shortcuts are recorded, reviewed and saved locally.
use crate::{
    app::{DialogKind, Intent, Rocket, UiControl},
    external_tools::ToolIntent,
    folder_browser::PickTarget,
    i18n::{mark, tr, trf},
    pages,
    style::*,
};
use eframe::egui::*;
use std::collections::HashMap;

/// Row heights of the density choices, as in View → Appearance.
pub(crate) const ROW_DENSITIES: [(&str, f32, &str); 3] = [
    ("rows-compact", 29.0, mark("Compact")),
    ("rows-comfortable", 33.0, mark("Comfortable")),
    ("rows-spacious", 38.0, mark("Spacious")),
];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Page {
    #[default]
    General,
    Git,
    Commands,
    Security,
    Appearance,
    Shortcuts,
    Terminal,
    Commit,
    Tools,
    Accounts,
    SshKeys,
    Integrations,
    Mcp,
    Storage,
}

impl Page {
    /// In sidebar order: each group's pages together, see [`Page::group`].
    pub const ALL: [Page; 14] = [
        Page::General,
        Page::Appearance,
        Page::Shortcuts,
        Page::Terminal,
        Page::Git,
        Page::Commit,
        Page::Commands,
        Page::Tools,
        Page::Accounts,
        Page::SshKeys,
        Page::Mcp,
        Page::Integrations,
        Page::Security,
        Page::Storage,
    ];
    /// The sidebar heading the page is listed under.
    fn group(self) -> &'static str {
        match self {
            Page::General | Page::Appearance | Page::Shortcuts | Page::Terminal => tr("WORKSPACE"),
            Page::Git | Page::Commit | Page::Commands | Page::Tools => tr("GIT"),
            Page::Accounts | Page::SshKeys | Page::Mcp => tr("CONNECTIONS"),
            Page::Integrations => tr("EXTENSIONS"),
            Page::Security | Page::Storage => tr("SECURITY & DATA"),
        }
    }
    pub fn label(self) -> &'static str {
        tr(self.english_label())
    }
    /// The page's name in English, which a settings search also reads.
    fn english_label(self) -> &'static str {
        match self {
            Page::General => mark("General"),
            Page::Git => mark("Git"),
            Page::Commands => mark("Custom commands"),
            Page::Security => mark("Security"),
            Page::Appearance => mark("Appearance"),
            Page::Shortcuts => mark("Shortcuts"),
            Page::Terminal => mark("Terminal"),
            Page::Commit => mark("Commit messages"),
            Page::Tools => mark("External tools"),
            Page::Accounts => mark("Accounts"),
            Page::SshKeys => mark("SSH keys"),
            Page::Integrations => mark("Extensions"),
            Page::Mcp => mark("MCP server"),
            Page::Storage => mark("Storage"),
        }
    }
    fn description(self) -> &'static str {
        match self {
            Page::General => tr("Window, updates, repositories and how diffs and commits read."),
            Page::Git => tr("The Git program Rocket runs, and your commit identity for every repository."),
            Page::Commands => tr("Your own commands in the Repository menu and right-click menus."),
            Page::Security => tr("Protect what you publish. Stay aware of dependency risks."),
            Page::Appearance => tr("Theme and row density."),
            Page::Shortcuts => tr("Customize keyboard shortcuts and mouse gestures for Rocket's commands."),
            Page::Terminal => tr("The shell the Terminal drawer runs."),
            Page::Commit => tr("Spelling and AI drafts."),
            Page::Tools => tr("Programs Rocket opens for files, comparisons and conflicts."),
            Page::Accounts => tr("Hosting accounts for pull requests, issues and publishing."),
            Page::SshKeys => tr("Choose or create a key, then copy its public key to your Git host."),
            Page::Integrations => tr("Sandboxed Lua packages with native panels."),
            Page::Mcp => tr("Let local tools connect to Rocket's control server."),
            Page::Storage => tr("Where Rocket keeps logs and saved state."),
        }
    }
    fn icon(self) -> Icon {
        match self {
            Page::General => Icon::Sliders,
            Page::Git => Icon::Branch,
            Page::Commands => Icon::Console,
            Page::Security => Icon::Lock,
            Page::Appearance => Icon::Contrast,
            Page::Shortcuts => Icon::Keyboard,
            Page::Terminal => Icon::Prompt,
            Page::Commit => Icon::Pencil,
            Page::Tools => Icon::External,
            Page::Accounts => Icon::User,
            Page::SshKeys => Icon::Key,
            Page::Integrations => Icon::Puzzle,
            Page::Mcp => Icon::Plug,
            Page::Storage => Icon::Database,
        }
    }
    /// The automation name of the page's sidebar entry.
    pub fn key(self) -> &'static str {
        match self {
            Page::General => "general",
            Page::Git => "git",
            Page::Commands => "commands",
            Page::Security => "security",
            Page::Appearance => "appearance",
            Page::Shortcuts => "shortcuts",
            Page::Terminal => "terminal",
            Page::Commit => "commit",
            Page::Tools => "tools",
            Page::Accounts => "accounts",
            Page::SshKeys => "ssh",
            Page::Integrations => "integrations",
            Page::Mcp => "mcp",
            Page::Storage => "storage",
        }
    }
    /// The name in the sidebar: Extensions' first row sits under a heading of that name.
    fn nav_label(self) -> &'static str {
        if self == Page::Integrations { tr("All extensions") } else { self.label() }
    }
    /// Whether a settings search finds this page.
    fn matches(self, query: &str) -> bool {
        let words = match self {
            Page::General => mark("language sidebar tabs repositories commit details activity drawer refresh fetch automatic window updates source code folder clone new repository tags submodules checkout branch names space hyphen push after commit diff marks font size syntax highlighting compact labels"),
            Page::Git => mark("git executable instance path program global name email identity"),
            Page::Commands => mark("custom commands program arguments script scope repository commit file branch submodule"),
            Page::Security => mark("security secret secrets protection github token tokens private key credentials .env commit push block dependencies dependency advisory advisories vulnerable library osv offline scan warnings database"),
            Page::Appearance => mark("theme default dark light system custom picture background accent colour color density rows compact comfortable spacious"),
            Page::Shortcuts => mark("keyboard shortcuts keys keybindings hotkeys mouse buttons wheel gestures"),
            Page::Terminal => mark("shell bash git bash cmd command prompt powershell pwsh console drawer program arguments"),
            Page::Commit => mark("spelling spell check ai draft claude codex anthropic openai language style conventional"),
            Page::Tools => mark("external editor diff merge program arguments profile placeholder beyond compare kdiff3 winmerge vs code visual studio code meld p4merge tortoisegitmerge sublime notepad++ araxis"),
            Page::Accounts => mark("hosting accounts sign in browser token github enterprise gitlab bitbucket azure devops gitea server profile connect"),
            Page::SshKeys => mark("ssh keys openssh agent passphrase identity profile ed25519 rsa public key repository unlock fingerprint copy found new add file"),
            Page::Integrations => mark("lua scripts integrations jira extensions plugins packages install update custom ui panels connections accounts permissions playground"),
            Page::Mcp => mark("mcp server port token endpoint fixture test repositories automation"),
            Page::Storage => mark("storage logs diagnostics saved state folder path explorer"),
        };
        // Either language finds a page: the words it shows, or their English.
        [format!("{} {words}", self.english_label()), format!("{} {}", self.label(), tr(words))]
            .iter()
            .any(|text| text.to_lowercase().contains(query))
            // Searching for an action also finds the page that lists its keys.
            || (self == Page::Shortcuts && !shortcut_groups(query).is_empty())
    }
}

#[derive(Default)]
pub struct State {
    pub page: Page,
    pub search: String,
    /// Closing was refused because tool profile or custom command edits are unsaved.
    pub unsaved_warning: bool,
    /// Shortcuts: the words or keys to find.
    pub shortcut_filter: String,
    /// Appearance: the accent colour code while it is typed.
    pub accent_code: String,
    /// Accounts was opened from the Hosting page: closing Preferences, or connecting,
    /// goes back there.
    pub hosting_return: bool,
    /// The page whose first reads (saved accounts, SSH profiles) were started; once per
    /// visit, so a failed read is not retried every frame.
    started: Option<Page>,
    /// General: the source code folder as it is typed, and its check.
    pub(crate) source_text: String,
    pub(crate) source_check: crate::repository_forms::Form,
    /// General: the sample the Diff view card draws.
    pub(crate) diff_preview: Option<crate::patch_view::Preview>,
}

/// The shortcut list, in groups: (keys, what they do). Keys use `+` within a
/// chord and ` / ` between alternatives.
const SHORTCUTS: [(&str, &[(&str, &str)]); 4] = [
    (
        mark("Repositories"),
        &[
            ("Ctrl+N", mark("New repository")),
            ("Ctrl+O", mark("Open repository")),
            ("Ctrl+Shift+O", mark("Clone repository")),
            ("Ctrl+W", mark("Close repository")),
            ("Ctrl+Shift+T", mark("Reopen closed repository")),
            ("Ctrl+Tab", mark("Next repository")),
            ("Ctrl+Shift+Tab", mark("Previous repository")),
        ],
    ),
    (
        mark("Panels"),
        &[
            ("Ctrl+K", mark("Command palette")),
            ("Ctrl+B", mark("Sidebar")),
            ("Ctrl+`", mark("Activity")),
            ("Ctrl+Shift+`", mark("Terminal")),
            ("Ctrl+,", mark("Preferences")),
        ],
    ),
    (
        mark("Working"),
        &[
            ("Ctrl+Enter", mark("Commit staged changes")),
            ("Ctrl+Shift+B", mark("New branch")),
            ("F5", mark("Refresh")),
        ],
    ),
    (
        mark("Lists and dialogs"),
        &[
            ("Up / Down", mark("Previous / next commit")),
            ("Space", mark("Commit details")),
            ("Tab / Shift+Tab", mark("Next / previous pane")),
            ("Enter / Space", mark("Activate the focused control")),
            ("Esc", mark("Close a menu or dialog")),
        ],
    ),
];

/// The groups with the shortcuts whose action or keys contain `filter`, ignoring
/// case; groups left empty are left out.
fn shortcut_groups(filter: &str) -> Vec<(&'static str, Vec<(&'static str, &'static str)>)> {
    let needle = filter.trim().to_lowercase();
    SHORTCUTS
        .iter()
        .map(|(title, items)| {
            let shown = items
                .iter()
                .filter(|(keys, action)| {
                    needle.is_empty()
                        || action.to_lowercase().contains(&needle)
                        || tr(action).to_lowercase().contains(&needle)
                        || keys.to_lowercase().contains(&needle)
                })
                .copied()
                .collect::<Vec<_>>();
            (*title, shown)
        })
        .filter(|(_, items)| !items.is_empty())
        .collect()
}

/// A titled group of settings rows on a card.
pub(crate) fn card<R>(ui: &mut Ui, caption: &str, content: impl FnOnce(&mut Ui) -> R) -> R {
    if !caption.is_empty() {
        ui.label(RichText::new(caption.to_uppercase()).font(semibold(11.0)).color(HINT()));
        ui.add_space(6.0);
    }
    let inner = Frame::new()
        .fill(CARD())
        .stroke(Stroke::new(1.0, CARD_STROKE()))
        .corner_radius(8)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            content(ui)
        })
        .inner;
    ui.add_space(22.0);
    inner
}

/// The line between two rows of a card.
pub(crate) fn divider(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter()
        .hline(rect.x_range(), rect.center().y, Stroke::new(1.0, CARD_LINE()));
}

pub(crate) fn keycap(ui: &mut Ui, text: &str) {
    let galley = ui
        .painter()
        .layout_no_wrap(text.into(), FontId::proportional(11.0), MUTED());
    let (rect, _) = ui.allocate_exact_size(vec2(galley.size().x + 12.0, 18.0), Sense::hover());
    ui.painter()
        .rect_stroke(rect, 4.0, Stroke::new(1.0, BORDER()), StrokeKind::Inside);
    ui.painter().galley(
        pos2(rect.left() + 6.0, rect.center().y - galley.size().y / 2.0),
        galley,
        MUTED(),
    );
}

/// A card row: a label with an optional hint and shortcut, and a switch at the right.
pub(crate) fn switch_row(
    ui: &mut Ui,
    widgets: &mut HashMap<String, UiControl>,
    id: &str,
    value: &mut bool,
    label: &str,
    hint: &str,
    shortcut: &str,
) -> Response {
    inset_switch_row(ui, 16.0, widgets, id, value, label, hint, shortcut)
}

/// [`switch_row`] on a header-bar card, its label in line with the bar's title.
pub(crate) fn bar_switch_row(
    ui: &mut Ui,
    widgets: &mut HashMap<String, UiControl>,
    id: &str,
    value: &mut bool,
    label: &str,
    hint: &str,
    shortcut: &str,
) -> Response {
    inset_switch_row(ui, 14.0, widgets, id, value, label, hint, shortcut)
}

#[allow(clippy::too_many_arguments)]
fn inset_switch_row(
    ui: &mut Ui,
    inset: f32,
    widgets: &mut HashMap<String, UiControl>,
    id: &str,
    value: &mut bool,
    label: &str,
    hint: &str,
    shortcut: &str,
) -> Response {
    let width = ui.available_width();
    let height = if hint.is_empty() { 46.0 } else { 56.0 };
    ui.allocate_ui_with_layout(
        vec2(width, height),
        Layout::left_to_right(Align::Center),
        |ui| {
            ui.add_space(inset);
            // Only as tall as its text, so the row centres it beside the switch.
            let text = vec2((width - 96.0).max(80.0), if hint.is_empty() { 20.0 } else { 38.0 });
            ui.allocate_ui_with_layout(text, Layout::top_down(Align::Min), |ui| {
                ui.set_width(text.x);
                ui.spacing_mut().item_spacing.y = 2.0;
                ui.spacing_mut().interact_size.y = 0.0;
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    ui.label(RichText::new(label).size(13.0));
                    if !shortcut.is_empty() {
                        let hint = crate::shortcuts::hint(ui.ctx(), shortcut);
                        if !hint.is_empty() {
                            ui.add(Label::new(RichText::new(&hint).monospace().small().color(HINT())).truncate()).on_hover_text(hint);
                        }
                    }
                });
                if !hint.is_empty() {
                    ui.add(Label::new(RichText::new(hint).size(12.0).color(HINT())).truncate());
                }
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add_space(inset);
                let r = switch(ui, value, label);
                pages::control(widgets, id, &r, ui.clip_rect());
                r
            })
            .inner
        },
    )
    .inner
}

/// A labelled card row: the label in a column that narrows with the card, the
/// control beside it.
pub(crate) fn field_row<R>(ui: &mut Ui, label: &str, content: impl FnOnce(&mut Ui) -> R) -> R {
    Frame::new()
        .inner_margin(Margin::symmetric(16, 9))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            let wanted = ui
                .painter()
                .layout_no_wrap(label.into(), FontId::proportional(13.0), MUTED())
                .size()
                .x
                + 12.0;
            let column = (ui.available_width() * 0.24).clamp(84.0, 150.0).max(wanted);
            ui.horizontal_top(|ui| {
                let (rect, _) = ui.allocate_exact_size(vec2(column, 28.0), Sense::hover());
                ui.painter().text(
                    rect.left_center(),
                    Align2::LEFT_CENTER,
                    label,
                    FontId::proportional(13.0),
                    MUTED(),
                );
                ui.vertical(|ui| {
                    ui.set_width(ui.available_width());
                    ui.spacing_mut().item_spacing.y = 5.0;
                    content(ui)
                })
                .inner
            })
            .inner
        })
        .inner
}

/// A one-line status under a field: a check in green or a notice in amber or red.
pub(crate) fn status_line(ui: &mut Ui, color: Color32, text: &str) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let (rect, _) = ui.allocate_exact_size(vec2(12.0, 12.0), Sense::hover());
        icon(
            ui.painter(),
            rect,
            if color == GREEN() { Icon::Check } else { Icon::Info },
            color,
        );
        ui.add(Label::new(RichText::new(text).size(12.0).color(color)).wrap());
    });
}

/// A card with a header bar, its rows edge to edge below the bar.
pub(crate) fn card_frame(ui: &mut Ui, content: impl FnOnce(&mut Ui)) {
    Frame::new()
        .fill(PANEL())
        .stroke(Stroke::new(1.0, CARD_STROKE()))
        .corner_radius(8)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            content(ui);
        });
    ui.add_space(16.0);
}

/// The card's header bar: its title, and `side` from the right edge.
pub(crate) fn card_header(ui: &mut Ui, title: &str, side: impl FnOnce(&mut Ui)) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 40.0), Sense::hover());
    ui.painter()
        .rect_filled(rect, CornerRadius { nw: 7, ne: 7, sw: 0, se: 0 }, CARD());
    ui.painter()
        .hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0, CARD_LINE()));
    // Drawn into the rectangle already taken, so the rows below start after the bar.
    let mut bar = ui.new_child(
        UiBuilder::new()
            .max_rect(rect.shrink2(vec2(14.0, 0.0)))
            .layout(Layout::right_to_left(Align::Center)),
    );
    bar.spacing_mut().item_spacing.x = 8.0;
    side(&mut bar);
    bar.with_layout(Layout::left_to_right(Align::Center), |ui| {
        ui.add(Label::new(RichText::new(title).font(semibold(13.0))).truncate());
    });
}

/// Messages at the foot of a card, inset like its rows.
pub(crate) fn card_note(ui: &mut Ui, content: impl FnOnce(&mut Ui)) {
    Frame::new()
        .inner_margin(Margin::symmetric(14, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 6.0;
            content(ui);
        });
}

/// A card's last row below a hairline: `content` from the right edge, its actions first and
/// then a note in the space left. `tint` fills the row, as for unsaved edits.
pub(crate) fn card_foot(ui: &mut Ui, tint: Option<Color32>, content: impl FnOnce(&mut Ui)) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 46.0), Sense::hover());
    if let Some(tint) = tint {
        ui.painter()
            .rect_filled(rect, CornerRadius { nw: 0, ne: 0, sw: 7, se: 7 }, tint);
    }
    ui.painter()
        .hline(rect.x_range(), rect.top() + 0.5, Stroke::new(1.0, CARD_LINE()));
    let mut row = ui.new_child(
        UiBuilder::new()
            .max_rect(rect.shrink2(vec2(14.0, 0.0)))
            .layout(Layout::right_to_left(Align::Center)),
    );
    row.spacing_mut().item_spacing.x = 8.0;
    content(&mut row);
}

/// A labelled row of a card: the label in a column `width` wide, then the field or value.
pub(crate) fn text_row(ui: &mut Ui, label: &str, width: f32, content: impl FnOnce(&mut Ui)) {
    Frame::new()
        .inner_margin(Margin::symmetric(14, 4))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 12.0;
                let (rect, _) = ui.allocate_exact_size(vec2(width, CONTROL), Sense::hover());
                ui.painter().text(
                    rect.left_center(),
                    Align2::LEFT_CENTER,
                    label,
                    FontId::proportional(13.0),
                    MUTED(),
                );
                content(ui);
            });
        });
}

/// A segmented control whose options are published under `ids`.
pub(crate) fn choices(
    ui: &mut Ui,
    widgets: &mut HashMap<String, UiControl>,
    items: &[(&str, bool)],
    ids: &[String],
) -> Option<usize> {
    let mut picked = None;
    for (i, r) in pages::segments(ui, items).into_iter().enumerate() {
        if let Some(id) = ids.get(i) {
            pages::control(widgets, id.clone(), &r, ui.clip_rect());
        }
        if r.clicked() {
            picked = Some(i);
        }
    }
    picked
}

fn nav_row(ui: &mut Ui, page: Page, selected: bool, rail: bool, dot: bool) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click());
    let hot = response.hovered() || response.has_focus();
    if selected {
        ui.painter().rect_filled(rect, 6.0, SELECTED());
        ui.painter().rect_filled(
            Rect::from_min_size(rect.min + vec2(0.0, 8.0), vec2(2.5, rect.height() - 16.0)),
            1.0,
            BLUE(),
        );
    } else if hot {
        ui.painter().rect_filled(rect, 6.0, HOVER());
    }
    let icon_center = if rail {
        rect.center()
    } else {
        pos2(rect.left() + 20.0, rect.center().y)
    };
    icon(
        ui.painter(),
        Rect::from_center_size(icon_center, vec2(15.0, 15.0)),
        page.icon(),
        if selected { BLUE() } else { HINT() },
    );
    if !rail {
        ui.painter().text(
            pos2(rect.left() + 38.0, rect.center().y),
            Align2::LEFT_CENTER,
            page.nav_label(),
            if selected { semibold(13.0) } else { FontId::proportional(13.0) },
            if selected || hot { TEXT() } else { TEXT_SOFT() },
        );
    }
    if dot {
        let center = if rail {
            pos2(rect.right() - 12.0, rect.top() + 10.0)
        } else {
            pos2(rect.right() - 14.0, rect.center().y)
        };
        ui.painter().circle_filled(center, 3.5, GREEN());
    }
    response.widget_info(|| WidgetInfo::selected(Role::Button, true, selected, page.nav_label()));
    if rail {
        response.on_hover_text(page.nav_label())
    } else {
        response
    }
}

/// An enabled extension's row: its letter tile where other rows have an icon.
fn extension_nav_row(ui: &mut Ui, entry: &crate::integrations::Entry, selected: bool, rail: bool) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click());
    let hot = response.hovered() || response.has_focus();
    if selected {
        ui.painter().rect_filled(rect, 6.0, SELECTED());
        ui.painter().rect_filled(
            Rect::from_min_size(rect.min + vec2(0.0, 8.0), vec2(2.5, rect.height() - 16.0)),
            1.0,
            BLUE(),
        );
    } else if hot {
        ui.painter().rect_filled(rect, 6.0, HOVER());
    }
    let center = if rail { rect.center() } else { pos2(rect.left() + 20.0, rect.center().y) };
    crate::integrations::letter_tile(
        ui.painter(),
        Rect::from_center_size(center, vec2(18.0, 18.0)),
        entry.letter,
        if selected { BLUE() } else { MUTED() },
    );
    if !rail {
        let color = if selected || hot { TEXT() } else { TEXT_SOFT() };
        let font = if selected { semibold(13.0) } else { FontId::proportional(13.0) };
        let mut job = text::LayoutJob::simple_singleline(entry.name.clone(), font, color);
        job.wrap = text::TextWrapping::truncate_at_width((rect.width() - 46.0).max(0.0));
        let name = ui.painter().layout_job(job);
        ui.painter().galley(pos2(rect.left() + 38.0, rect.center().y - name.size().y / 2.0), name, color);
    }
    response.widget_info(|| WidgetInfo::selected(Role::Button, true, selected, &entry.name));
    if rail { response.on_hover_text(&entry.name) } else { response }
}

impl Rocket {
    /// Edit → Preferences, opened at `page`.
    pub(crate) fn show_preferences(&mut self, page: Page) {
        self.preferences.page = page;
        if page == Page::Security {
            self.dependencies.setup_page = false;
        }
        self.integrations.show_list();
        self.preferences.search.clear();
        self.preferences.shortcut_filter.clear();
        self.preferences.unsaved_warning = false;
        self.preferences.hosting_return = false;
        self.preferences.started = None;
        self.show_dialog(DialogKind::Preferences);
    }

    /// Hosting → Accounts: Preferences at Accounts, then back to Hosting on close or
    /// once an account connects. An error that sent you here stays visible.
    pub(crate) fn show_accounts_from_hosting(&mut self) {
        self.forge.section = crate::forge::Section::Accounts;
        self.forge.preview = None;
        self.show_preferences(Page::Accounts);
        self.preferences.hosting_return = true;
    }

    /// Reopen Hosting at its lists, keeping any message from the Accounts page.
    fn return_to_hosting(&mut self) {
        if self.forge.section == crate::forge::Section::Accounts {
            self.forge.section = crate::forge::Section::Pulls;
        }
        self.show_dialog(DialogKind::Hosting);
    }

    /// Whether Preferences is open at `page` (the MCP host reports the MCP page).
    pub fn preferences_page(&self) -> Option<Page> {
        self.dialog
            .as_ref()
            .filter(|d| matches!(d.kind, DialogKind::Preferences))
            .map(|_| self.preferences.page)
    }

    pub(crate) fn preferences_dialog(&mut self, ctx: &Context) {
        let width = (ctx.content_rect().width() - 48.0).clamp(520.0, 1000.0);
        let height = (ctx.content_rect().height() - 48.0).clamp(420.0, 720.0);
        let rail = width < 760.0;
        if self.preferences.page == Page::Mcp {
            self.mcp.open();
        }
        if self.preferences.started != Some(self.preferences.page) {
            self.preferences.started = Some(self.preferences.page);
            match self.preferences.page {
                Page::Accounts => self.start_forge_page(),
                Page::SshKeys => self.start_ssh_page(),
                Page::General => self.start_general_page(),
                Page::Terminal => self.start_terminal_page(),
                Page::Tools => self.external.page_opened(),
                _ => {}
            }
        }
        let mut done = false;
        let mut save = false;
        let mut discard = false;
        let (mut commands_save, mut commands_discard) = (false, false);
        let (mut shell_save, mut shell_discard, mut open_git) = (false, false, false);
        let mut intents = Vec::new();
        let mut browse = None;
        let response = Modal::new(Id::unique("rocket-preferences"))
            .backdrop_color(Color32::from_black_alpha(120))
            .frame(pages::modal_frame())
            .show(ctx, |ui| {
                ui.set_width(width);
                ui.set_height(height);
                let outer = ui.max_rect();
                let nav = Rect::from_min_size(outer.min, vec2(if rail { 64.0 } else { 224.0 }, outer.height()));
                let main = Rect::from_min_max(pos2(nav.right(), outer.top()), outer.max);
                ui.painter().rect_filled(
                    nav,
                    CornerRadius { nw: 11, sw: 11, ne: 0, se: 0 },
                    NAV(),
                );
                ui.painter()
                    .vline(nav.right() - 0.5, nav.y_range(), Stroke::new(1.0, HAIRLINE()));
                ui.scope_builder(UiBuilder::new().max_rect(nav.shrink2(vec2(10.0, 16.0))), |ui| {
                    self.preferences_nav(ui, rail);
                });
                let page = self.preferences.page;
                // There is no Done: settings apply as they change and × closes. The footer
                // holds Save/Discard for tool profiles and a line on how settings are kept,
                // and is left out where it has nothing to say.
                // Git keeps its Save and Stop on its cards; Custom commands always has a
                // footer for Save commands.
                let note = match page {
                    Page::Tools => tr("Profiles are saved on this computer."),
                    Page::Terminal => tr("Applies to the next terminal."),
                    Page::Commands => "",
                    Page::Accounts if cfg!(windows) => tr("Remembered accounts are protected for your Windows user."),
                    Page::Accounts => tr("Remembered accounts are encrypted with a key in your desktop keyring."),
                    Page::SshKeys => tr("Key files and your own SSH config are never overwritten."),
                    Page::Git | Page::Mcp | Page::Storage | Page::Shortcuts | Page::Integrations => "",
                    _ => tr("Changes apply right away."),
                };
                let unsaved = self.external.editing != self.external.profiles;
                let extension_form = page == Page::Integrations && self.integrations.form_open();
                let footer_height = if unsaved || extension_form || !note.is_empty() || page == Page::Commands { 56.0 } else { 0.0 };
                // Header: the page's title and purpose, the guide and close.
                let header = Rect::from_min_size(main.min, vec2(main.width(), 72.0));
                ui.painter().hline(
                    header.x_range(),
                    header.bottom() - 0.5,
                    Stroke::new(1.0, HAIRLINE()),
                );
                ui.scope_builder(
                    UiBuilder::new()
                        .max_rect(header.shrink2(vec2(24.0, 16.0)))
                        .layout(Layout::right_to_left(Align::Min)),
                    |ui| {
                        let close = crate::chrome::icon_button(ui, Icon::Close, true, false, 30.0)
                            .on_hover_text(tr("Close (Esc)"));
                        pages::control(&mut self.frame_widgets, "dialog-close", &close, ui.clip_rect());
                        // Automation left with the Done button, and on these two pages with
                        // their old back buttons; Done was hidden while profiles were unsaved.
                        if !(unsaved && page == Page::Tools) {
                            pages::control(&mut self.frame_widgets, "dialog-submit", &close, ui.clip_rect());
                            match page {
                                Page::Tools => pages::control(&mut self.frame_widgets, "tools-back", &close, ui.clip_rect()),
                                Page::Mcp => pages::control(&mut self.frame_widgets, "mcp-back", &close, ui.clip_rect()),
                                // The Close buttons of the former SSH keys and Hosting pages.
                                Page::Accounts | Page::SshKeys => pages::control(&mut self.frame_widgets, "dialog-cancel", &close, ui.clip_rect()),
                                _ => {}
                            }
                        }
                        done |= close.clicked();
                        if crate::guide::available(ui.ctx()) {
                            crate::guide::header_button(ui);
                        }
                        ui.with_layout(Layout::top_down(Align::Min), |ui| {
                            ui.spacing_mut().item_spacing.y = 3.0;
                            ui.label(RichText::new(page.label()).font(semibold(18.0)));
                            ui.add(Label::new(RichText::new(page.description()).color(MUTED())).truncate());
                        });
                    },
                );
                let footer = Rect::from_min_max(pos2(main.left(), main.bottom() - footer_height), main.max);
                let body = Rect::from_min_max(pos2(main.left(), header.bottom()), pos2(main.right(), footer.top()));
                ui.scope_builder(UiBuilder::new().max_rect(body), |ui| {
                    let scroll = ScrollArea::vertical()
                        .id_salt(("preferences", if page == Page::Security && self.dependencies.setup_page { "security-setup" } else { page.key() }))
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            Frame::new()
                                .inner_margin(Margin {
                                    left: 28,
                                    right: 28,
                                    top: 18,
                                    bottom: 6,
                                })
                                .show(ui, |ui| {
                                    ui.set_width(ui.available_width());
                                    ui.spacing_mut().item_spacing.y = 6.0;
                                    match self.preferences.page {
                                        Page::General => {
                                            if self.general_page(ui) {
                                                browse = Some(PickTarget::SourceFolder);
                                            }
                                        }
                                        Page::Git => { if self.git_settings_page(ui) { browse = Some(PickTarget::GitProgram); } },
                                        Page::Commands => {
                                            // The working copy lives with this dialog, so a page switch keeps it.
                                            if let Some(d) = self.dialog.as_mut() {
                                                let e = d.commands.get_or_insert_with(|| {
                                                    let mut e = crate::custom_commands::Editor::new(self.commands.clone());
                                                    e.selected = None;
                                                    e
                                                });
                                                let log = self.backend.as_ref().map(|b| b.log.clone());
                                                browse = browse.or(crate::custom_commands::page(ui, e, &mut self.frame_widgets, self.browse_root.as_deref(), log.as_ref()));
                                            }
                                        }
                                        Page::Security => self.security_page(ui),
                                        Page::Appearance => {
                                            if self.appearance_page(ui) {
                                                browse = Some(PickTarget::ThemePicture);
                                            }
                                        }
                                        Page::Shortcuts => self.shortcuts_page(ui),
                                        Page::Commit => {
                                            if self.commit_page(ui) {
                                                browse = Some(PickTarget::DraftProgram);
                                            }
                                        }
                                        Page::Tools => browse = browse.or(self.tools_page(ui)),
                                        Page::Terminal => {
                                            if self.terminal_page(ui, &mut open_git) {
                                                browse = Some(PickTarget::ShellProgram);
                                            }
                                        }
                                        Page::Accounts => self.accounts_preferences(ui),
                                        Page::SshKeys => self.ssh_preferences(ui),
                                        Page::Integrations => browse = browse.or(self.integration_settings(ui)),
                                        Page::Mcp => {
                                            if self.mcp.page(ui, &mut self.frame_widgets) {
                                                browse = Some(PickTarget::McpRoot);
                                            }
                                        }
                                        Page::Storage => self.storage_page(ui, &mut intents),
                                    }
                                });
                        });
                    let scroll = scroll.inner_rect.intersect(ui.clip_rect());
                    pages::region(&mut self.frame_widgets, "prefs-scroll", scroll, true);
                    // The scroll areas of the pages these settings came from.
                    let alias = match self.preferences.page {
                        Page::Tools => Some("tools-scroll"),
                        Page::Accounts => Some("hosting-scroll"),
                        Page::SshKeys => Some("ssh-scroll"),
                        // The former Custom commands dialog's scroll area.
                        Page::Commands => Some("dialog-scroll"),
                        _ => None,
                    };
                    if let Some(alias) = alias {
                        pages::region(&mut self.frame_widgets, alias, scroll, true);
                    }
                });
                if footer_height > 0.0 {
                    ui.painter().rect_filled(
                        footer,
                        CornerRadius { nw: 0, ne: 0, sw: 0, se: 11 },
                        FOOTER(),
                    );
                    ui.painter()
                        .hline(footer.x_range(), footer.top(), Stroke::new(1.0, HAIRLINE()));
                    // Read again: the Tools page may have just been edited.
                    let dirty = self.external.editing != self.external.profiles;
                    let shell_dirty = page == Page::Terminal && self.terminal_shell.dirty();
                    ui.scope_builder(
                        UiBuilder::new()
                            .max_rect(footer.shrink2(vec2(24.0, 12.0)))
                            .layout(Layout::right_to_left(Align::Center)),
                        |ui| {
                            ui.spacing_mut().item_spacing.x = 8.0;
                            let busy = self.external.running();
                            // Accounts and SSH keys keep their pages' automation names for Stop.
                            if page == Page::SshKeys && self.ssh.running() {
                                let r = ui.add_enabled(!self.ssh.stopping, Button::new(if self.ssh.stopping { tr("Stopping") } else { tr("Stop") }));
                                pages::control(&mut self.frame_widgets, "ssh-cancel", &r, ui.clip_rect());
                                if r.clicked() {
                                    self.ssh.cancel();
                                }
                            }
                            if page == Page::Accounts && self.forge.running() {
                                let r = ui.add_enabled(!self.forge.stopping, Button::new(if self.forge.stopping { tr("Stopping") } else { tr("Stop") }));
                                pages::control(&mut self.frame_widgets, "hosting-cancel", &r, ui.clip_rect());
                                if r.clicked() {
                                    self.forge.cancel();
                                }
                            }
                            if extension_form {
                                if let Some(backend) = self.backend.clone() {
                                    let writable = self.profile_storage_ready();
                                    self.integrations.form_footer(ui, &mut self.frame_widgets, backend, writable);
                                }
                                return;
                            }
                            if page == Page::Commands {
                                if let Some(e) = self.dialog.as_ref().and_then(|d| d.commands.as_ref()) {
                                    (commands_save, commands_discard) = crate::custom_commands::footer(ui, e, &mut self.frame_widgets, self.preferences.unsaved_warning);
                                }
                                return;
                            }
                            if dirty && page == Page::Tools {
                                let r = pages::primary(ui, !busy, tr("Save profiles"));
                                pages::control(&mut self.frame_widgets, "tools-save-settings", &r, ui.clip_rect());
                                save = r.clicked();
                                let r = ui.add_enabled(!busy, Button::new(tr("Discard")));
                                pages::control(&mut self.frame_widgets, "tools-discard-settings", &r, ui.clip_rect());
                                discard = r.clicked();
                            }
                            if shell_dirty {
                                let r = pages::primary(ui, self.terminal_shell.savable, tr("Save"));
                                pages::control(&mut self.frame_widgets, "shell-save", &r, ui.clip_rect());
                                shell_save = r.clicked();
                                let r = ui.add(Button::new(tr("Discard")));
                                pages::control(&mut self.frame_widgets, "shell-discard", &r, ui.clip_rect());
                                shell_discard = r.clicked();
                            }
                            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                                if dirty || shell_dirty {
                                    let (rect, _) = ui.allocate_exact_size(vec2(8.0, 8.0), Sense::hover());
                                    ui.painter().circle_filled(rect.center(), 3.5, AMBER());
                                    let text = match (shell_dirty, self.preferences.unsaved_warning) {
                                        (true, true) => tr("Save or discard the shell change first."),
                                        (true, false) => tr("Unsaved shell change"),
                                        (false, true) => tr("Save or discard the tool profile changes first."),
                                        (false, false) => tr("Unsaved tool profile changes"),
                                    };
                                    ui.add(Label::new(RichText::new(text).size(12.5).color(AMBER())).truncate());
                                } else if (page == Page::SshKeys && self.ssh.running()) || (page == Page::Accounts && self.forge.running()) {
                                    let stopping = if page == Page::SshKeys { self.ssh.stopping } else { self.forge.stopping };
                                    ui.spinner();
                                    let text = match (page, stopping) {
                                        (_, true) => tr("Stopping; checking the result"),
                                        (Page::SshKeys, false) => tr("Working with OpenSSH"),
                                        _ => tr("Waiting for the hosting service"),
                                    };
                                    ui.label(RichText::new(text).size(12.0).color(HINT()));
                                } else {
                                    ui.label(RichText::new(note).size(12.0).color(HINT()));
                                }
                            });
                        },
                    );
                }
            });
        done |= response.should_close();
        if save {
            self.external_intent(ToolIntent::SaveSettings);
            self.preferences.unsaved_warning = false;
        }
        if discard {
            self.external.reset_edits();
            self.external.error = None;
            self.external.notice = Some(tr("Unsaved profile edits discarded.").into());
            self.preferences.unsaved_warning = false;
        }
        if shell_save {
            self.terminal_shell.save();
            self.preferences.unsaved_warning = false;
        }
        if shell_discard {
            self.terminal_shell.discard();
            self.preferences.unsaved_warning = false;
        }
        if open_git {
            self.preferences.page = Page::Git;
        }
        if commands_save || commands_discard {
            if let Some(e) = self.dialog.as_mut().and_then(|d| d.commands.as_mut()) {
                if commands_discard {
                    e.discard();
                } else if e.problem().is_none() && e.changed() {
                    self.commands = e.list.clone();
                    e.mark_saved();
                }
            }
            self.preferences.unsaved_warning = false;
        }
        // Edits the page holds, or an import or export still running, keep Preferences open.
        let commands_open = self
            .dialog
            .as_ref()
            .and_then(|d| d.commands.as_ref())
            .is_some_and(|e| e.changed() || e.transfer.is_some());
        if done {
            if self.git_settings.running() || self.git_settings.dirty() {
                self.preferences.page = Page::Git;
                self.git_settings.hold_close();
            } else if commands_open {
                self.preferences.page = Page::Commands;
                self.preferences.unsaved_warning = true;
            } else if self.external.editing != self.external.profiles {
                // Leaving would hide edits that block launching; ask for a decision.
                self.preferences.page = Page::Tools;
                self.preferences.unsaved_warning = true;
            } else if self.terminal_shell.dirty() {
                self.preferences.page = Page::Terminal;
                self.preferences.unsaved_warning = true;
            } else if self.ssh.running() {
                // OpenSSH may be waiting for a passphrase that only this page can send.
                self.preferences.page = Page::SshKeys;
                self.ssh.notice = Some(tr("Wait for OpenSSH to finish, or cancel it, before closing.").into());
            } else {
                self.dialog = None;
                self.preferences.started = None;
                if std::mem::take(&mut self.preferences.hosting_return) {
                    self.return_to_hosting();
                }
            }
        } else if self.preferences.hosting_return
            && self.preferences.page == Page::Accounts
            && self.forge.session.is_some()
            && self.forge.section != crate::forge::Section::Accounts
        {
            // Connecting moves Hosting on to its lists: the account was what Hosting needed.
            self.dialog = None;
            self.preferences.started = None;
            self.preferences.hosting_return = false;
            self.return_to_hosting();
        }
        if let Some(target) = browse {
            let current = match target {
                PickTarget::GitProgram => self.git_settings.browse_start(),
                PickTarget::DraftProgram => self.drafting.program_setting().to_owned(),
                PickTarget::AiProgram(provider) => self.drafting.program_setting_of(provider).to_owned(),
                PickTarget::McpRoot => self.mcp.root.clone(),
                PickTarget::SourceFolder => self.general.source_folder.as_ref().map(|p| p.display().to_string()).unwrap_or_default(),
                PickTarget::ToolProgram(kind, index) => self.external.editing.0[kind.index()]
                    .entries
                    .get(index)
                    .map(|p| p.config.executable.clone())
                    .unwrap_or_default(),
                PickTarget::ShellProgram => self.terminal_shell.browse_start(),
                PickTarget::CommandProgram(_) | PickTarget::CommandImport | PickTarget::CommandExport => {
                    crate::custom_commands::browse_start(self.dialog.as_ref().and_then(|d| d.commands.as_ref()), target)
                }
                // Clone, New repository, Run command, Submodules and the repository manager
                // browse from their own dialogs; a theme picture starts in the Pictures folder.
                PickTarget::CloneSource
                | PickTarget::DependencyScanner
                | PickTarget::ExtensionFile
                | PickTarget::LibraryRepository
                | PickTarget::LibraryScanFolder
                | PickTarget::CrashReportFolder
                | PickTarget::MapExportFolder
                | PickTarget::CloneParent
                | PickTarget::InitFolder
                | PickTarget::WorktreeFolder
                | PickTarget::SeriesFolder
                | PickTarget::SeriesSave
                | PickTarget::CommandInput(_)
                | PickTarget::SubmoduleSource
                | PickTarget::ThemePicture => {
                    String::new()
                }
            };
            self.browse(target, &current);
        }
        for intent in intents {
            self.dispatch(intent);
        }
    }

    fn preferences_nav(&mut self, ui: &mut Ui, rail: bool) {
        ui.spacing_mut().item_spacing.y = 2.0;
        if !rail {
            ui.label(RichText::new(tr("Preferences")).font(semibold(15.0)));
            ui.add_space(8.0);
            let search = search_field(ui, &mut self.preferences.search, tr("Search settings"), ui.available_width());
            pages::control(&mut self.frame_widgets, "pref-search", &search, ui.clip_rect());
            ui.add_space(4.0);
        }
        let query = self.preferences.search.trim().to_lowercase();
        let visible: Vec<Page> = Page::ALL
            .into_iter()
            .filter(|p| query.is_empty() || p.matches(&query))
            .collect();
        if let Some(first) = visible.first()
            && !visible.contains(&self.preferences.page)
        {
            self.preferences.page = *first;
        }
        // Fourteen pages in five groups: the list scrolls when the window is short.
        // The version is in Help → About, not here.
        ScrollArea::vertical()
            .id_salt("pref-nav")
            .max_height(ui.available_height().max(60.0))
            .auto_shrink([false, true])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                let mut group = "";
                for page in &visible {
                    if page.group() != group {
                        // The group's heading; the rail separates groups with a line instead.
                        if rail {
                            if !group.is_empty() {
                                ui.add_space(4.0);
                                let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
                                ui.painter().hline(rect.shrink2(vec2(8.0, 0.0)).x_range(), rect.center().y, Stroke::new(1.0, HAIRLINE()));
                                ui.add_space(4.0);
                            }
                        } else {
                            ui.add_space(if group.is_empty() { 0.0 } else { 4.0 });
                            ui.horizontal(|ui| {
                                ui.add_space(10.0);
                                ui.label(RichText::new(page.group()).font(semibold(11.0)).color(FAINT()));
                            });
                        }
                        group = page.group();
                    }
                    let dot = *page == Page::Mcp && self.mcp.status.running;
                    let open = self.integrations.details_entry();
                    let r = nav_row(ui, *page, *page == self.preferences.page && !(*page == Page::Integrations && open.is_some()), rail, dot);
                    pages::control(&mut self.frame_widgets, format!("pref-page-{}", page.key()), &r, ui.clip_rect());
                    if *page == Page::Security {
                        // Keep the former Dependencies navigation target for existing automation.
                        pages::control(&mut self.frame_widgets, "pref-page-dependencies", &r, ui.clip_rect());
                    }
                    if *page == Page::Tools {
                        // The Preferences button that used to open the External tools page.
                        pages::control(&mut self.frame_widgets, "pref-tools", &r, ui.clip_rect());
                    }
                    if r.clicked() {
                        self.preferences.page = *page;
                        self.integrations.show_list();
                        if *page == Page::Tools && self.external.editing == self.external.profiles && !self.external.pending() {
                            self.external.selected = self.external.profiles.selections();
                        }
                    }
                    if *page == Page::Integrations {
                        for entry in self.integrations.entries().to_vec() {
                            let selected = self.preferences.page == Page::Integrations && open.as_deref() == Some(entry.id.as_str());
                            let r = extension_nav_row(ui, &entry, selected, rail);
                            pages::control(&mut self.frame_widgets, format!("pref-ext-{}", entry.id), &r, ui.clip_rect());
                            if r.clicked() {
                                self.preferences.page = Page::Integrations;
                                if let Some(backend) = self.backend.clone() {
                                    self.integrations.show_details(backend, entry.id);
                                }
                            }
                        }
                    }
                }
                if visible.is_empty() && !rail {
                    ui.add_space(6.0);
                    ui.label(RichText::new(tr("No settings match.")).size(12.0).color(HINT()));
                }
            });
    }

    /// Returns whether Browse was clicked for the source code folder.
    fn general_page(&mut self, ui: &mut Ui) -> bool {
        let widgets = &mut self.frame_widgets;
        card_frame(ui, |ui| {
            card_header(ui, tr("Window"), |_| {});
            crate::general::language_row(ui, widgets, &mut self.language);
            divider(ui);
            bar_switch_row(ui, widgets, "pref-sidebar", &mut self.show_sidebar, tr("Show sidebar"), "", "Ctrl+B");
            divider(ui);
            bar_switch_row(
                ui,
                widgets,
                "pref-repository-list",
                &mut self.repository_list,
                tr("List open repositories in the sidebar"),
                tr("Instead of tabs across the top"),
                "",
            );
            divider(ui);
            bar_switch_row(ui, widgets, "pref-details", &mut self.show_details, tr("Show commit details"), "", "");
            divider(ui);
            let r = bar_switch_row(ui, widgets, "pref-console", &mut self.show_console, tr("Show the Activity drawer"), "", "Ctrl+`");
            // Turning it on asks for Activity: the whole drawer, not a collapsed strip.
            if r.changed() && self.show_console {
                self.drawer_collapsed = false;
            }
        });
        card_frame(ui, |ui| {
            card_header(ui, tr("Updates"), |_| {});
            bar_switch_row(
                ui,
                widgets,
                "pref-auto-refresh",
                &mut self.auto_refresh,
                tr("Refresh local changes automatically"),
                tr("Picks up edits made outside Rocket"),
                "",
            );
            divider(ui);
            crate::auto_fetch::preferences(ui, &mut self.auto_fetch.minutes, widgets);
        });
        self.general_cards(ui)
    }

    fn shortcuts_page(&mut self, ui: &mut Ui) {
        self.shortcut_preferences(ui);
    }

    /// Returns whether Browse was clicked for the draft program.
    fn commit_page(&mut self, ui: &mut Ui) -> bool {
        if crate::spell::Spelling::supported() {
            let hint = if !self.spell_check {
                if cfg!(windows) {
                    tr("Uses the Windows spell checker.").to_owned()
                } else {
                    if cfg!(target_os = "macos") { tr("Uses the macOS spell checker. Added words are shared with other apps.").to_owned() }
                    else { tr("Uses the Hunspell dictionaries installed on this system.").to_owned() }
                }
            } else {
                match (&self.spelling.problem, self.spelling.language()) {
                    (Some(problem), _) => trf!("Off: {problem}", problem),
                    (None, Some(language)) if cfg!(windows) => trf!("Windows spell checker · {language}", language),
                    (None, Some(language)) if cfg!(target_os = "macos") => trf!("macOS spell checker · {language}", language),
                    (None, Some(language)) => trf!("Hunspell dictionary · {language}", language),
                    (None, None) => tr("Starting the spell checker…").to_owned(),
                }
            };
            let widgets = &mut self.frame_widgets;
            let spell_check = &mut self.spell_check;
            card(ui, tr("Spelling"), |ui| {
                switch_row(ui, widgets, "pref-spell-check", spell_check, tr("Check spelling as you type"), &hint, "");
            });
        }
        let widgets = &mut self.frame_widgets;
        let drafting = &mut self.drafting;
        card(ui, tr("AI drafts"), |ui| crate::commit_draft::preferences(ui, drafting, widgets))
    }

    fn storage_page(&mut self, ui: &mut Ui, intents: &mut Vec<Intent>) {
        let rows = [
            (
                tr("Diagnostic logs"),
                self.log_path.clone(),
                tr("Five files of up to 1 MiB; the oldest records go first."),
                "logs",
            ),
            (
                tr("Saved state"),
                self.state_directory.clone(),
                tr("Window layout, recent repositories and these preferences."),
                "state",
            ),
        ];
        let widgets = &mut self.frame_widgets;
        card(ui, "", |ui| {
            for (i, (label, path, hint, key)) in rows.iter().enumerate() {
                if i > 0 {
                    divider(ui);
                }
                Frame::new().inner_margin(Margin::symmetric(16, 12)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.set_width((ui.available_width() - 250.0).max(160.0));
                            ui.spacing_mut().item_spacing.y = 3.0;
                            ui.label(RichText::new(*label).font(semibold(13.0)));
                            ui.add(Label::new(RichText::new(path.display().to_string()).size(12.0).monospace().color(TEXT_SOFT())).wrap());
                            ui.label(RichText::new(*hint).size(12.0).color(HINT()));
                        });
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let show = pages::tool(ui, Icon::Folder, tr("Show in Explorer"), true);
                            pages::control(widgets, format!("pref-show-{key}"), &show, ui.clip_rect());
                            if show.clicked() {
                                intents.push(Intent::ShowFolder(path.clone()));
                            }
                            let copy = pages::tool(ui, Icon::Copy, tr("Copy path"), true);
                            pages::control(widgets, format!("pref-copy-{key}"), &copy, ui.clip_rect());
                            if copy.clicked() {
                                ui.ctx().copy_text(path.display().to_string());
                            }
                        });
                    });
                });
            }
        });
        if let Some(error) = &self.state_error {
            pages::callout(ui, AMBER(), error);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_common as common;

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
                app.dialog_ui(&ctx);
                app.picker_dialog(&ctx);
            },
        );
        output.textures_delta.clear();
    }

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
    fn language_popup_accepts_separate_pointer_press_and_release() {
        common::isolated(
            "ROCKET_LANGUAGE_POINTER_CASE",
            "preferences::tests::language_popup_accepts_separate_pointer_press_and_release",
            || {
                let fixture = common::Fixture::new();
                let ctx = Context::default();
                let mut app = Rocket::with_log_directory(&ctx, vec![], fixture.root.join("logs"));
                crate::batch::flow_tests::wait(&ctx, &mut app);
                app.show_preferences(Page::General);
                frame(&ctx, &mut app, vec![]);
                frame(&ctx, &mut app, vec![]);
                click(&ctx, &mut app, "pref-language");
                frame(&ctx, &mut app, vec![]);
                assert!(!app.frame_widgets.contains_key("pref-language-system"));
                let modal_layer = ctx.layer_id_at(app.frame_widgets["pref-language"].rect.center());
                for language in crate::i18n::offered() {
                    let control = &app.frame_widgets[&format!("pref-language-{}", language.tag().to_ascii_lowercase())];
                    assert!(control.enabled);
                    let top_at_option = ctx.layer_id_at(control.rect.center());
                    assert!(top_at_option.is_some() && top_at_option != modal_layer, "{} must be above the Preferences modal", language.tag());
                }
                click(&ctx, &mut app, "pref-language-zh-hans");
                assert_eq!(app.language, Some(crate::i18n::Language::ChineseSimplified));
                assert_eq!(app.preferences_page(), Some(Page::General));
                click(&ctx, &mut app, "pref-language");
                frame(&ctx, &mut app, vec![]);
                click(&ctx, &mut app, "pref-language-en");
                assert_eq!(app.language, Some(crate::i18n::Language::English));
                click(&ctx, &mut app, "pref-language");
                frame(&ctx, &mut app, vec![Event::Key {
                    key: Key::Escape, physical_key: None, pressed: true,
                    repeat: false, modifiers: Modifiers::NONE,
                }]);
                frame(&ctx, &mut app, vec![Event::Key {
                    key: Key::Escape, physical_key: None, pressed: false,
                    repeat: false, modifiers: Modifiers::NONE,
                }]);
                assert_eq!(app.preferences_page(), Some(Page::General));
                assert!(!app.frame_widgets.contains_key("pref-language-ja"));
                assert_eq!(app.language, Some(crate::i18n::Language::English));
            },
        );
    }

    #[test]
    fn shortcuts_are_found_by_action_or_keys() {
        let groups = |filter: &str| {
            shortcut_groups(filter).iter().map(|(title, items)| (*title, items.len())).collect::<Vec<_>>()
        };
        assert_eq!(groups(""), [("Repositories", 7), ("Panels", 5), ("Working", 3), ("Lists and dialogs", 5)]);
        assert_eq!(groups(" TERMINAL "), [("Panels", 1)]);
        assert_eq!(groups("ctrl+shift+o"), [("Repositories", 1)]);
        assert!(groups("no such action").is_empty());
        // Searching settings for an action finds the page that lists its keys.
        assert!(Page::Shortcuts.matches("terminal"));
        assert!(Page::Shortcuts.matches("keyboard"));
        assert!(!Page::General.matches("terminal"));
    }

    #[test]
    fn the_terminal_page_saves_only_on_save_and_holds_the_dialog_until_then() {
        common::isolated(
            "ROCKET_PREFERENCES_UI_CASE",
            "preferences::tests::the_terminal_page_saves_only_on_save_and_holds_the_dialog_until_then",
            || {
                use crate::{batch::flow_tests::wait, terminal_settings::{Choice, Settings}};
                // A shell that is always there, other than the default.
                let (other, other_key) = if cfg!(windows) { (Choice::CommandPrompt, "command-prompt") } else { (Choice::Bash, "bash") };
                let other_id = format!("shell-choice-{other_key}");
                let fixture = common::Fixture::new();
                let ctx = Context::default();
                let mut app = Rocket::with_log_directory(&ctx, vec![], fixture.root.join("logs"));
                wait(&ctx, &mut app);
                app.dispatch(Intent::Preferences(Page::Terminal));
                frame(&ctx, &mut app, vec![]);
                frame(&ctx, &mut app, vec![]);
                for id in ["shell-choice", "shell-program", "shell-arguments"] {
                    assert!(app.frame_widgets.contains_key(id), "{id}");
                }
                assert!(!app.frame_widgets.contains_key("shell-save"), "nothing to save yet");
                click(&ctx, &mut app, "shell-choice");
                frame(&ctx, &mut app, vec![]);
                for choice in Choice::ALL {
                    let id = format!("shell-choice-{}", choice.key());
                    assert!(app.frame_widgets.contains_key(&id), "{id}");
                }
                click(&ctx, &mut app, &other_id);
                frame(&ctx, &mut app, vec![]);
                assert_eq!(app.terminal_shell.editing.choice, other);
                assert_eq!(app.terminal_shell.saved, Settings::default(), "choosing doesn't save");

                // Closing with an edit stays open and asks for a decision.
                app.show_preferences(Page::General);
                frame(&ctx, &mut app, vec![]);
                click(&ctx, &mut app, "dialog-close");
                frame(&ctx, &mut app, vec![]);
                assert_eq!(app.preferences_page(), Some(Page::Terminal));
                assert!(app.preferences.unsaved_warning);
                click(&ctx, &mut app, "shell-discard");
                frame(&ctx, &mut app, vec![]);
                assert_eq!(app.terminal_shell.editing, Settings::default());
                assert!(!app.frame_widgets.contains_key("shell-save"));

                click(&ctx, &mut app, "shell-choice");
                frame(&ctx, &mut app, vec![]);
                click(&ctx, &mut app, "shell-choice-custom");
                frame(&ctx, &mut app, vec![]);
                let had = if cfg!(windows) { "powershell.exe".to_owned() } else { std::env::var("SHELL").unwrap_or_default() };
                assert!(app.terminal_shell.editing.program.ends_with(&had), "Custom starts from the shell you had");
                assert!(app.frame_widgets.contains_key("shell-browse"));
                assert!(app.frame_widgets["shell-program"].enabled);

                // Locked, as in an MCP window: nothing on the card takes input.
                app.terminal_shell.discard();
                app.terminal_shell.editable = false;
                frame(&ctx, &mut app, vec![]);
                frame(&ctx, &mut app, vec![]);
                assert!(!app.frame_widgets["shell-choice"].enabled);
                app.terminal_shell.editable = true;
                frame(&ctx, &mut app, vec![]);

                click(&ctx, &mut app, "shell-choice");
                frame(&ctx, &mut app, vec![]);
                click(&ctx, &mut app, &other_id);
                frame(&ctx, &mut app, vec![]);
                frame(&ctx, &mut app, vec![]);
                click(&ctx, &mut app, "shell-save");
                assert_eq!(app.terminal_shell.saved.choice, other);
                click(&ctx, &mut app, "dialog-close");
                assert!(app.dialog.is_none(), "saved, so it closes");
            },
        );
    }

    #[test]
    fn add_lists_known_tools_and_choosing_one_only_fills_the_form() {
        common::isolated(
            "ROCKET_PREFERENCES_UI_CASE",
            "preferences::tests::add_lists_known_tools_and_choosing_one_only_fills_the_form",
            || {
                use crate::batch::flow_tests::wait;
                use rocket_git::ExternalToolKind;
                let fixture = common::Fixture::new();
                let ctx = Context::default();
                let mut app = Rocket::with_log_directory(&ctx, vec![], fixture.root.join("logs"));
                wait(&ctx, &mut app);
                app.show_preferences(Page::Tools);
                frame(&ctx, &mut app, vec![]);
                frame(&ctx, &mut app, vec![]);
                click(&ctx, &mut app, "tools-kind-2");
                frame(&ctx, &mut app, vec![]);
                click(&ctx, &mut app, "tools-profile-add");
                // A popup is measured on its first frame and shown on the next.
                frame(&ctx, &mut app, vec![]);
                frame(&ctx, &mut app, vec![]);
                for id in ["tools-known-vscode", "tools-known-beyond-compare", "tools-known-sublime-merge", "tools-known-empty"] {
                    assert!(app.frame_widgets.contains_key(id), "{id}");
                }
                assert!(!app.frame_widgets.contains_key("tools-known-notepad-plus-plus"), "an editor, not a merge tool");
                click(&ctx, &mut app, "tools-known-kdiff3");
                frame(&ctx, &mut app, vec![]);
                let merge = ExternalToolKind::Merge.index();
                let profile = &app.external.editing.0[merge].entries[0];
                assert_eq!(app.external.editing.0[merge].entries.len(), 1, "the empty Default took it");
                assert_eq!(profile.name, "KDiff3");
                assert_eq!(profile.config.arguments, "{base}\n{ours}\n{theirs}\n-o\n{result}");
                assert_eq!(app.external.profiles.0[merge].entries[0].name, "Default", "nothing saves yet");
                assert!(app.frame_widgets.contains_key("tools-save-settings"));

                // Empty profile is the plain Add it always was.
                click(&ctx, &mut app, "tools-profile-add");
                frame(&ctx, &mut app, vec![]);
                frame(&ctx, &mut app, vec![]);
                click(&ctx, &mut app, "tools-known-empty");
                frame(&ctx, &mut app, vec![]);
                assert_eq!(app.external.editing.0[merge].entries.len(), 2);
                assert_eq!(app.external.selected[merge], 1);
                app.external.reset_edits();
            },
        );
    }

    /// Keyboard shortcuts live in Preferences only: the palette's Shortcuts opens the
    /// whole list, × closes it, and Help does not list them.
    #[test]
    fn keyboard_shortcuts_open_in_preferences_with_the_whole_list() {
        common::isolated(
            "ROCKET_PREFERENCES_UI_CASE",
            "preferences::tests::keyboard_shortcuts_open_in_preferences_with_the_whole_list",
            || {
                use crate::batch::flow_tests::{click, enabled, wait};
                let fixture = common::Fixture::new();
                let ctx = Context::default();
                let mut app = Rocket::with_log_directory(&ctx, vec![], fixture.root.join("logs"));
                wait(&ctx, &mut app);
                app.preferences.page = Page::General;
                app.preferences.shortcut_filter = "earlier".into();
                // What the palette's Shortcuts entry dispatches.
                app.dispatch(Intent::Preferences(Page::Shortcuts));
                assert_eq!(app.preferences_page(), Some(Page::Shortcuts));
                assert!(app.preferences.shortcut_filter.is_empty(), "it opens with the whole list");
                assert!(enabled(&ctx, &mut app, "pref-shortcuts-filter"));
                click(&ctx, &mut app, "dialog-close");
                assert!(app.dialog.is_none());
                click(&ctx, &mut app, "menu-help");
                assert!(enabled(&ctx, &mut app, "help-guide"));
                assert!(!app.frame_widgets.contains_key("help-shortcuts"), "Help lists no shortcuts");
            },
        );
    }

    #[test]
    fn secret_protection_is_user_toggleable_but_mcp_controls_stay_disabled() {
        common::isolated("ROCKET_PREFERENCES_UI_CASE", "preferences::tests::secret_protection_is_user_toggleable_but_mcp_controls_stay_disabled", || {
            let fixture = common::Fixture::new();
            let ctx = Context::default();
            let mut app = Rocket::with_storage(&ctx, vec![], fixture.root.join("logs"), fixture.root.join("state"));
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
            while app.backend.is_none() || app.busy() {
                app.poll(&ctx);
                assert!(std::time::Instant::now() < deadline, "startup should settle");
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            assert!(app.secret_protection);
            app.show_preferences(Page::Security);
            frame(&ctx, &mut app, vec![]); frame(&ctx, &mut app, vec![]);
            assert!(app.frame_widgets["pref-secret-protection"].enabled);
            assert!(app.guard_ai_api_keys);
            assert!(app.frame_widgets["pref-ai-key-guard"].enabled);
            click(&ctx, &mut app, "pref-ai-key-guard");
            assert!(!app.guard_ai_api_keys);
            assert!(app.secret_protection, "AI guard does not toggle the existing guard");
            click(&ctx, &mut app, "pref-secret-protection");
            assert!(!app.secret_protection);
            app.poll(&ctx);
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
            while app.state_pending() {
                app.poll(&ctx);
                assert!(app.state_error.is_none(), "settings must save successfully");
                assert!(std::time::Instant::now() < deadline, "settings should finish saving");
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            drop(app);
            let mut app = Rocket::with_storage(&ctx, vec![], fixture.root.join("logs-reopened"), fixture.root.join("state"));
            while app.backend.is_none() || app.busy() {
                app.poll(&ctx);
                assert!(std::time::Instant::now() < deadline, "reopened app should settle");
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            assert!(app.state_error.is_none());
            assert!(!app.secret_protection, "the user's off choice survives restarting");
            assert!(!app.guard_ai_api_keys, "AI guard choice survives restarting");
            app.show_preferences(Page::Security);
            frame(&ctx, &mut app, vec![]); frame(&ctx, &mut app, vec![]);
            click(&ctx, &mut app, "pref-secret-protection");
            assert!(app.secret_protection);
            assert!(!app.guard_ai_api_keys);
            click(&ctx, &mut app, "pref-ai-key-guard");
            assert!(app.guard_ai_api_keys);
            app.secret_protection_editable = false;
            frame(&ctx, &mut app, vec![]);
            assert!(!app.frame_widgets["pref-secret-protection"].enabled);
            assert!(!app.frame_widgets["pref-ai-key-guard"].enabled);
            click(&ctx, &mut app, "pref-ai-key-guard");
            assert!(app.guard_ai_api_keys);
            click(&ctx, &mut app, "pref-secret-protection");
            assert!(app.secret_protection);
        });
    }

    #[test]
    fn pages_publish_their_controls_and_done_waits_for_tool_edits() {
        common::isolated(
            "ROCKET_PREFERENCES_UI_CASE",
            "preferences::tests::pages_publish_their_controls_and_done_waits_for_tool_edits",
            || {
                let fixture = common::Fixture::new();
                let ctx = Context::default();
                let mut app = Rocket::with_log_directory(&ctx, vec![], fixture.root.join("logs"));
                let pages: [(Page, &[&str]); 8] = [
                    (
                        Page::General,
                        &[
                            "pref-search", "pref-page-general", "pref-page-mcp", "pref-tools",
                            "pref-sidebar", "pref-repository-list", "pref-details", "pref-console",
                            "pref-auto-refresh", "pref-auto-fetch", "dialog-submit", "dialog-close",
                        ],
                    ),
                    (
                        Page::Security,
                        &[
                            "pref-page-security", "pref-page-dependencies", "pref-secret-protection",
                            "pref-dependency-warnings", "dependency-setup-open",
                        ],
                    ),
                    (
                        Page::Appearance,
                        &[
                            "theme-clay", "theme-ocean", "theme-dark", "theme-light", "theme-system", "theme-custom",
                            "rows-compact", "rows-comfortable", "rows-spacious",
                        ],
                    ),
                    (Page::Shortcuts, &["pref-page-shortcuts", "pref-shortcuts-filter"]),
                    (Page::Commit, &["pref-ai-enabled"]),
                    (
                        Page::Tools,
                        &[
                            "tools-kind-0", "tools-kind-1", "tools-kind-2", "tools-profile-0",
                            "tools-profile-picker", "tools-profile-add", "tools-profile-duplicate",
                            "tools-profile-name", "tools-executable", "tools-browse", "tools-arguments",
                            "tools-insert-0", "tools-profile-remove", "tools-back", "tools-scroll",
                        ],
                    ),
                    (
                        Page::Mcp,
                        &[
                            "mcp-status", "mcp-port", "mcp-root", "mcp-browse", "mcp-start",
                            "mcp-copy-token", "mcp-rotate", "mcp-back",
                        ],
                    ),
                    (Page::Storage, &["pref-copy-logs", "pref-show-logs", "pref-copy-state", "pref-show-state"]),
                ];
                for (page, ids) in pages {
                    app.show_preferences(page);
                    // A new modal is placed once its size is known, after one frame.
                    frame(&ctx, &mut app, vec![]);
                    frame(&ctx, &mut app, vec![]);
                    if page == Page::Mcp {
                        // The page's fields replace their placeholders once its first check answers.
                        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
                        while app.mcp.busy() {
                            app.poll(&ctx);
                            assert!(std::time::Instant::now() < deadline, "the MCP check did not finish");
                            std::thread::sleep(std::time::Duration::from_millis(10));
                        }
                        frame(&ctx, &mut app, vec![]);
                    }
                    for id in ids {
                        assert!(app.frame_widgets.contains_key(*id), "{} shows {id}", page.label());
                    }
                    if page == Page::General {
                        assert!(!app.frame_widgets.contains_key("pref-secret-protection"));
                        assert!(!app.frame_widgets.contains_key("pref-dependency-warnings"));
                    }
                }
                assert_eq!(app.preferences_page(), Some(Page::Storage));

                // Browse opens the folder picker above Preferences; its × returns to it.
                app.show_preferences(Page::Tools);
                frame(&ctx, &mut app, vec![]);
                click(&ctx, &mut app, "tools-browse");
                frame(&ctx, &mut app, vec![]);
                frame(&ctx, &mut app, vec![]);
                assert!(app.picker.is_some());
                assert!(app.frame_widgets.contains_key("picker-choose"));
                click(&ctx, &mut app, "picker-cancel");
                assert!(app.picker.is_none());
                assert!(app.dialog.is_some(), "cancelling the picker keeps Preferences");

                // No Done beside ×: its old id is the × itself.
                app.show_preferences(Page::General);
                frame(&ctx, &mut app, vec![]);
                assert_eq!(app.frame_widgets["dialog-submit"].rect, app.frame_widgets["dialog-close"].rect);

                // Unsaved tool profiles keep Preferences open until saved or discarded.
                app.external.editing.0[0].entries[0].name = "Edited".into();
                app.show_preferences(Page::General);
                frame(&ctx, &mut app, vec![]);
                click(&ctx, &mut app, "dialog-submit");
                assert!(app.dialog.is_some(), "× kept the unsaved profile edits");
                assert_eq!(app.preferences.page, Page::Tools);
                frame(&ctx, &mut app, vec![]);
                click(&ctx, &mut app, "tools-discard-settings");
                assert_eq!(app.external.editing, app.external.profiles);
                frame(&ctx, &mut app, vec![]);
                click(&ctx, &mut app, "dialog-submit");
                assert!(app.dialog.is_none());
            },
        );
    }

    #[test]
    fn security_navigation_and_search_find_both_protections() {
        common::isolated(
            "ROCKET_PREFERENCES_UI_CASE",
            "preferences::tests::security_navigation_and_search_find_both_protections",
            || {
                let fixture = common::Fixture::new();
                let ctx = Context::default();
                let mut app = Rocket::with_storage(&ctx, vec![], fixture.root.join("logs"), fixture.root.join("state"));
                app.show_preferences(Page::General);
                frame(&ctx, &mut app, vec![]);
                frame(&ctx, &mut app, vec![]);
                assert_eq!(app.frame_widgets["pref-page-security"].rect, app.frame_widgets["pref-page-dependencies"].rect);
                click(&ctx, &mut app, "pref-page-security");
                frame(&ctx, &mut app, vec![]);
                assert_eq!(app.preferences_page(), Some(Page::Security));
                assert!(app.frame_widgets.contains_key("pref-secret-protection"));
                assert!(app.frame_widgets.contains_key("pref-dependency-warnings"));
                for query in ["Security", "secrets", "tokens", ".env", "dependencies", "advisories", "OSV"] {
                    app.show_preferences(Page::General);
                    app.preferences.search = query.into();
                    frame(&ctx, &mut app, vec![]);
                    assert_eq!(app.preferences_page(), Some(Page::Security), "search: {query}");
                    assert!(app.frame_widgets.contains_key("pref-secret-protection"));
                    assert!(app.frame_widgets.contains_key("pref-dependency-warnings"));
                    assert!(!app.frame_widgets.contains_key("pref-page-general"));
                }
            },
        );
    }
}
