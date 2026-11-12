use crate::{app::*, backend::Action, i18n::{tr, trf}, pages, style::*, terminal::TerminalAction};
use eframe::egui::*;
use std::path::PathBuf;
#[cfg(target_os = "macos")]
mod native;
/// Icon, title and subtitle of a generic dialog's review header.
fn dialog_header(dialog: &Dialog) -> (Icon, String, String) {
    let (image, title, subtitle): (Icon, &str, String) = match &dialog.kind {
        DialogKind::Open => (
            Icon::Folder,
            tr("Open repository"),
            tr("Paste a folder path, or drop a folder into the main window.").into(),
        ),
        DialogKind::NewRepository => (
            Icon::Plus,
            tr("New repository"),
            tr("Start tracking a new or existing project with Git.").into(),
        ),
        DialogKind::Signing(_) => (Icon::Tag, tr("Signing"), "".into()),
        DialogKind::AtomicPush => (Icon::Push, tr("Push branches together"), "".into()),
        DialogKind::Ignore(_) => (Icon::File, tr("Manage .gitignore"), "".into()),
        DialogKind::Workflows => (Icon::Branch, tr("Branch workflows & cleanup"), "".into()),
        DialogKind::RepositorySettings => (
            Icon::Commit,
            tr("Repository settings"),
            tr("Saved in this repository only. Your global Git settings don't change.").into(),
        ),
        DialogKind::Clone => (
            Icon::Fetch,
            tr("Clone repository"),
            tr("Download a repository into a new folder.").into(),
        ),
        DialogKind::NewBranch => (
            Icon::Branch,
            tr("New branch"),
            if dialog.branch.is_empty() {
                tr("Make a branch from where you are.").into()
            } else {
                trf!("Make a branch from where you are. You stay on {branch}.", branch = display(&dialog.branch))
            },
        ),
        DialogKind::TrackBranch { upstream, .. } => (
            Icon::Branch,
            tr("New branch"),
            trf!("Make a branch from {upstream} that tracks it, then switch to it.", upstream = short_ref(upstream)),
        ),
        DialogKind::Rename(old) => (
            Icon::Branch,
            tr("Rename branch"),
            display(old),
        ),
        DialogKind::DeleteBranches => (
            Icon::Branch,
            tr("Delete branches"),
            tr("Local branches only. Branches on remotes stay.").into(),
        ),
        // The generic dialog names it after the command and its repository.
        DialogKind::RunCommand(_) => (
            Icon::Console,
            tr("Run custom command"),
            tr("Review the captured selection and exact action before continuing.").into(),
        ),
        DialogKind::AddRemote => (
            Icon::Remote,
            tr("Add remote"),
            tr("Connect another copy of this repository, such as the project you forked.").into(),
        ),
        DialogKind::EditRemote(remote, _) => (
            Icon::Remote,
            tr("Edit remote URL"),
            display(&remote.name),
        ),
        DialogKind::Fetch => (
            Icon::Fetch,
            tr("Fetch"),
            tr("Download what's new without touching your branches or files.").into(),
        ),
        DialogKind::Pull => (
            Icon::Pull,
            tr("Pull"),
            trf!("Pull the configured upstream into {branch}?", branch = display(&dialog.branch)),
        ),
        DialogKind::Push => (
            Icon::Push,
            tr("Push"),
            tr("Publish your commits to a branch on a remote.").into(),
        ),
        DialogKind::Stash => (
            Icon::Stash,
            tr("Save changes to stash"),
            tr("Set your changes aside and get a clean working folder.").into(),
        ),
        DialogKind::Merge => (
            Icon::Branch,
            tr("Merge branch"),
            tr("Merges another branch into the current branch.").into(),
        ),
        DialogKind::Confirm(action, message) => {
            let (image, title) = match action {
                Action::DeleteBranch(_) => (Icon::Branch, tr("Delete branch")),
                Action::RemoveRemote(_) => (Icon::Remote, tr("Remove remote")),
                Action::AbortMerge => (Icon::Branch, tr("Abort merge")),
                Action::FinishMerge => (Icon::Branch, tr("Clear completed merge state")),
                Action::DropStash(_) => (Icon::Stash, tr("Drop stash")),
                Action::ApplyStash { .. } => (Icon::Stash, tr("Apply stash")),
                Action::SaveConflict { stage: true, .. } => (Icon::File, tr("Save and stage resolution")),
                Action::SaveConflict { .. } => (Icon::File, tr("Save resolution")),
                Action::KeepConflictSide { keep, .. } => (
                    Icon::File,
                    match keep {
                        rocket_git::ConflictKeep::Ours => tr("Keep our version"),
                        rocket_git::ConflictKeep::Theirs => tr("Keep their version"),
                        rocket_git::ConflictKeep::Deletion => tr("Keep the deletion"),
                    },
                ),
                _ => (Icon::Check, tr("Confirm operation")),
            };
            // The conflict-save message has several paragraphs; it goes in the body.
            let subtitle = if matches!(
                action,
                Action::SaveConflict { .. } | Action::KeepConflictSide { .. }
            ) {
                String::new()
            } else {
                message.clone()
            };
            (image, title, subtitle)
        }
        DialogKind::ConflictReload(path) => (
            Icon::Refresh,
            tr("Reload conflict"),
            trf!("Reload {path} from the current working file and Git conflict stages?", path = path.display_lossy()),
        ),
        DialogKind::Preferences => (
            Icon::Layout,
            tr("Preferences"),
            tr("Drafts, recent repositories and preferences are saved locally.").into(),
        ),
        DialogKind::Legal => (
            Icon::File,
            tr("Legal"),
            tr("Licenses for Rocket and the open-source software it includes.").into(),
        ),
        DialogKind::About => (
            Icon::Rocket,
            tr("About Rocket"),
            tr("A native Rust Git workspace.").into(),
        ),
        DialogKind::Changelog => (
            Icon::File,
            tr("Release notes"),
            tr("Changes in each Rocket development build.").into(),
        ),
        DialogKind::CloseTab(_) => (
            Icon::Folder,
            tr("Close repository"),
            tr("Its message draft is saved for next time.").into(),
        ),
        DialogKind::Quit => (
            Icon::Rocket,
            tr("Exit Rocket"),
            tr("Commit drafts and settings will be saved before exit.").into(),
        ),
        DialogKind::RepositoryManager
        | DialogKind::Explore
        | DialogKind::KeepPreview(_)
        | DialogKind::UndoCommit
        | DialogKind::CommitAction(_)
        | DialogKind::Rebase(_)
        | DialogKind::MailImport(_)
        | DialogKind::Bisect
        | DialogKind::Recovery
        | DialogKind::Tags
        | DialogKind::Worktrees
        | DialogKind::Submodules
        | DialogKind::Lfs
        | DialogKind::GitHub
        | DialogKind::Hosting
        | DialogKind::Integrations
        | DialogKind::Statistics
        | DialogKind::Benchmark
        | DialogKind::FileEdit(_)
        | DialogKind::GitFlow(_)
        | DialogKind::DraftNotice(_)
        | DialogKind::Patch(_) => unreachable!("separate reviewed operation dialog"),
    };
    // Confirmations name what they act on.
    let (title, subtitle) = match &dialog.kind {
        DialogKind::Confirm(Action::DropStash(entry), _) => (
            trf!("Drop {stash}?", stash = format!("stash@{{{}}}", entry.index)),
            display(&entry.subject),
        ),
        DialogKind::Confirm(Action::ApplyStash { entry, .. }, _) => (
            trf!("Apply {stash}?", stash = format!("stash@{{{}}}", entry.index)),
            display(&entry.subject),
        ),
        DialogKind::Confirm(Action::DeleteBranch(name), _) => (
            trf!("Delete {branch}?", branch = display(name)),
            tr("Local branch only. Branches on remotes stay.").into(),
        ),
        DialogKind::Confirm(Action::RemoveRemote(remote), _) => (
            trf!("Remove remote {remote}?", remote = display(&remote.name)),
            tr("Its address and remote branches leave this repository.").into(),
        ),
        DialogKind::Quit => (tr("Exit Rocket?").into(), tr("Commit drafts and settings are saved first.").into()),
        _ => (title.into(), subtitle),
    };
    (image, title, subtitle)
}
fn item(ui: &mut Ui, label: &str, shortcut: &str, enabled: bool) -> bool {
    item_response(ui, label, shortcut, enabled).clicked()
}
pub(crate) fn item_response(ui: &mut Ui, label: &str, shortcut: &str, enabled: bool) -> Response {
    let hint = crate::shortcuts::hint(ui.ctx(), shortcut);
    hinted_item(ui, label, &hint, enabled)
}
/// A row whose shortcut text its caller already resolved, as for the optional review
/// bindings of Fetch, Pull and Push.
fn hinted_item(ui: &mut Ui, label: &str, hint: &str, enabled: bool) -> Response {
    let response = menu_row(ui, label, hint, enabled);
    if response.clicked() {
        ui.close();
    }
    response
}
// A hovered, focused or open menu row sits on MENU_HOT with MENU_HOT_TEXT, its
// shortcut or submenu marker in TEXT_SOFT; a hovered menu-bar title sits on
// MENU_TITLE_HOT, an open one on the selection colour.

/// One menu row: label inset from the left, shortcut (or submenu marker) on the right.
/// Long labels (recent paths, tab titles) widen the row instead of being clipped.
/// Commands show without a trailing ellipsis; see `command_label`.
fn menu_row(ui: &mut Ui, label: &str, right: &str, enabled: bool) -> Response {
    menu_row_lit(ui, label, right, enabled, false)
}
/// A menu row that is also lit while `open` (its submenu shows).
fn menu_row_lit(ui: &mut Ui, label: &str, right: &str, enabled: bool, open: bool) -> Response {
    let label = command_label(label);
    let text = ui
        .painter()
        .layout_no_wrap(label.into(), FontId::proportional(13.5), TEXT());
    let shortcut = ui
        .painter()
        .layout_no_wrap(right.into(), FontId::proportional(12.0), FAINT());
    let width = (12.0 + text.size().x + 24.0 + shortcut.size().x + 10.0).max(300.0);
    // The band sits under the text, so its slot is taken before anything is painted.
    let band = ui.painter().add(Shape::Noop);
    let response = ui.add_enabled(
        enabled,
        Button::new("").frame(false).min_size(vec2(width, 30.0)),
    );
    response.widget_info(|| WidgetInfo::labeled(Role::Button, enabled, label));
    let rect = response.rect;
    let lit = enabled && (open || response.hovered() || response.has_focus());
    if lit {
        ui.painter()
            .set(band, Shape::rect_filled(rect, 6.0, MENU_HOT()));
    }
    let (ink, key) = if !enabled {
        (FAINT().gamma_multiply(0.8), FAINT())
    } else if lit {
        (MENU_HOT_TEXT(), TEXT_SOFT())
    } else {
        (TEXT(), FAINT())
    };
    // Text only: per-item icons were mostly the same generic file glyph.
    ui.painter().galley_with_override_text_color(
        pos2(rect.left() + 12.0, rect.center().y - text.size().y / 2.0),
        text,
        ink,
    );
    ui.painter().galley_with_override_text_color(
        pos2(
            rect.right() - 10.0 - shortcut.size().x,
            rect.center().y - shortcut.size().y / 2.0,
        ),
        shortcut,
        key,
    );
    response
}
/// A menu-bar title: a bright band and white text while hovered, the selection colour
/// while its menu is open. egui reports a menu open only after drawing its title, so the
/// title follows the previous frame's state.
fn menu_title<R>(
    ui: &mut Ui,
    title: &str,
    content: impl FnOnce(&mut Ui) -> R,
) -> InnerResponse<Option<R>> {
    let open_id = menu_open_id(ui, title);
    let was_open = menu_was_open(ui, title);
    let response = ui
        .scope(|ui| {
            let visuals = ui.visuals_mut();
            // Per-state text colours apply only without the app-wide override.
            visuals.override_text_color = None;
            let widgets = &mut visuals.widgets;
            widgets.inactive.fg_stroke.color = TEXT();
            widgets.hovered.weak_bg_fill = MENU_TITLE_HOT();
            widgets.hovered.fg_stroke.color = MENU_HOT_TEXT();
            widgets.active.weak_bg_fill = SELECTED();
            widgets.active.fg_stroke.color = MENU_HOT_TEXT();
            if was_open {
                widgets.inactive.weak_bg_fill = SELECTED();
                widgets.inactive.fg_stroke.color = MENU_HOT_TEXT();
                widgets.hovered.weak_bg_fill = SELECTED();
            }
            ui.menu_button(title, content)
        })
        .inner;
    ui.data_mut(|d| d.insert_temp(open_id, response.inner.is_some()));
    response
}
/// A toggle or choice row. It keeps the label on the same inset as every other row
/// (egui's checkboxes and radios indent theirs) and marks the selection on the right.
fn check_row(ui: &mut Ui, label: &str, checked: bool, enabled: bool) -> Response {
    let response = menu_row(ui, label, "", enabled);
    response.widget_info(|| WidgetInfo::selected(Role::CheckBox, enabled, checked, label));
    if checked {
        let mark = Rect::from_center_size(
            pos2(response.rect.right() - 16.0, response.rect.center().y),
            vec2(12.0, 12.0),
        );
        icon(
            ui.painter(),
            mark,
            Icon::Check,
            if enabled { BLUE() } else { FAINT() },
        );
    }
    if response.clicked() {
        ui.close();
    }
    response
}
/// A small caption above a group of rows, on the same inset as their labels.
fn menu_caption(ui: &mut Ui, text: &str) {
    let galley = ui
        .painter()
        .layout_no_wrap(text.into(), FontId::proportional(11.5), FAINT());
    // Sized to the text: a popup's available width is far wider than its rows.
    let (rect, _) = ui.allocate_exact_size(vec2(12.0 + galley.size().x, 22.0), Sense::hover());
    ui.painter().galley(
        pos2(rect.left() + 12.0, rect.center().y - galley.size().y / 2.0),
        galley,
        FAINT(),
    );
}
/// A submenu drawn like the other rows, so its label lines up with theirs
/// (egui's default submenu button uses its own, narrower padding). A disabled
/// submenu is greyed like its siblings and does not open.
fn submenu(ui: &mut Ui, label: &str, enabled: bool, content: impl FnOnce(&mut Ui)) {
    let (response, open_id) = submenu_row(ui, label, enabled);
    let open = enabled && menu::SubMenu::new().show(ui, &response, content).is_some();
    ui.data_mut(|d| d.insert_temp(open_id, open));
}
/// A submenu's row. It stays lit while the submenu shows, which egui reports only after
/// the row, so the caller stores whether it showed under the returned id.
fn submenu_row(ui: &mut Ui, label: &str, enabled: bool) -> (Response, Id) {
    let open_id = ui.scope_id().with(("submenu-open", label));
    let was_open = enabled && ui.data(|d| d.get_temp::<bool>(open_id)).unwrap_or(false);
    // egui 0.37 paints its own submenu arrow as a triangle and dropped the glyph constant;
    // Rocket's rows keep the glyph they have always shown.
    let response = menu_row_lit(ui, label, "⏵", enabled, was_open);
    (response, open_id)
}
type Widgets = std::collections::HashMap<String, UiControl>;
/// `submenu`, with its row published under `key` so automation can open it. Its rows
/// get the same map back to publish theirs.
fn keyed_submenu(
    ui: &mut Ui,
    widgets: &mut Widgets,
    key: &str,
    label: &str,
    enabled: bool,
    content: impl FnOnce(&mut Ui, &mut Widgets),
) {
    let (row, open_id) = submenu_row(ui, label, enabled);
    pages::control(widgets, key, &row, ui.clip_rect());
    let open = enabled && menu::SubMenu::new().show(ui, &row, |ui| content(ui, widgets)).is_some();
    ui.data_mut(|d| d.insert_temp(open_id, open));
}
/// A menu row published under `key` for automation; true when it was chosen.
fn keyed_item(ui: &mut Ui, widgets: &mut Widgets, key: &str, label: &str, shortcut: &str, enabled: bool) -> bool {
    let response = item_response(ui, label, shortcut, enabled);
    pages::control(widgets, key, &response, ui.clip_rect());
    response.clicked()
}
/// Repository → Extensions: what extensions offer without a target. It shows only when
/// there is something to list.
fn extensions_menu(ui: &mut Ui, widgets: &mut Widgets, opened: bool, intents: &mut Vec<Intent>) {
    let menu = crate::integrations::menu(ui.ctx());
    let enabled = menu.busy.is_none();
    if !menu.entries.iter().any(|e| e.commands_for(None).next().is_some()) {
        return;
    }
    keyed_submenu(ui, widgets, "repository-extensions", tr("Extensions"), opened, |ui, widgets| {
        for entry in menu.entries.iter().filter(|e| e.commands_for(None).next().is_some()) {
            menu_caption(ui, &entry.name);
            if let Some(fix) = entry.fix_for(None) {
                if keyed_item(ui, widgets, &format!("ext-menu-{}", entry.id), &fix.label(&entry.name), "", enabled) {
                    intents.push(Intent::ExtensionFix(entry.id.clone(), fix));
                }
                continue;
            }
            for command in entry.commands_for(None) {
                if keyed_item(ui, widgets, &format!("ext-menu-{}-{}", entry.id, command.id), &command.label, "", enabled) {
                    intents.push(Intent::ExtensionCommand(entry.run(command, None)));
                }
            }
        }
        ui.separator();
        if keyed_item(ui, widgets, "extensions-manage", tr("Manage Extensions"), "", true) {
            intents.push(Intent::Preferences(crate::preferences::Page::Integrations));
        }
    });
}
/// Where `menu_title` records whether its menu showed.
fn menu_open_id(ui: &Ui, title: &str) -> Id {
    ui.scope_id().with(("menu-open", title))
}
/// Whether the menu-bar menu `title` showed last frame.
fn menu_was_open(ui: &Ui, title: &str) -> bool {
    ui.data(|d| d.get_temp::<bool>(menu_open_id(ui, title))).unwrap_or(false)
}
/// The text field an Edit menu command acts on, with the selection it had when the menu
/// opened: egui collapses a field's selection as soon as it loses focus.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct EditTarget {
    field: Id,
    selection: Option<text::CCursorRange>,
}
/// The focused widget, if it is a text field, with its selection.
fn focused_text_field(ctx: &Context) -> Option<EditTarget> {
    let field = ctx.memory(|m| m.focused())?;
    let state = TextEdit::load_state(ctx, field)?;
    Some(EditTarget { field, selection: state.cursor.char_range() })
}
/// What an Edit menu item does to the text field the menu was opened from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TextCommand {
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    SelectAll,
}
impl TextCommand {
    /// The event the field gets in place of the keyboard shortcut. Paste has none: egui
    /// reads the clipboard and sends its text (`ViewportCommand::RequestPaste`).
    fn event(self) -> Option<Event> {
        // The platform's command key, as egui-winit reports it.
        let modifiers = if cfg!(target_os = "macos") {
            Modifiers::MAC_CMD | Modifiers::COMMAND
        } else {
            Modifiers::CTRL | Modifiers::COMMAND
        };
        let key = |key| Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        };
        match self {
            Self::Undo => Some(key(Key::Z)),
            Self::Redo => Some(key(Key::Y)),
            Self::SelectAll => Some(key(Key::A)),
            Self::Cut => Some(Event::Cut),
            Self::Copy => Some(Event::Copy),
            Self::Paste => None,
        }
    }
}
impl Rocket {
    /// View → Appearance: its row and the submenu of themes and row densities.
    pub(crate) fn appearance_menu(&mut self, ui: &mut Ui) {
        let (theme, row_height) = (&mut self.theme, &mut self.row_height);
        keyed_submenu(ui, &mut self.frame_widgets, "view-appearance", tr("Appearance"), true, |ui, widgets| {
            for choice in crate::theme::Choice::ALL {
                let r = check_row(ui, choice.label(), theme.choice == choice, true);
                pages::control(widgets, format!("view-theme-{}", choice.key()), &r, ui.clip_rect());
                if r.clicked() {
                    theme.choice = choice;
                }
            }
            ui.separator();
            for (key, value, label) in crate::preferences::ROW_DENSITIES {
                let r = check_row(ui, &trf!("{label} rows", label = tr(label)), *row_height == value, true);
                pages::control(widgets, key, &r, ui.clip_rect());
                if r.clicked() {
                    *row_height = value;
                }
            }
        });
    }
    /// Destructive decisions get the red header and button.
    fn dialog_danger(&self, dialog: &Dialog) -> bool {
        let unsaved = |id: Option<u64>| {
            self.tabs
                .iter()
                .any(|t| Some(t.id) == id && t.conflict.dirty)
        };
        match &dialog.kind {
            DialogKind::Confirm(
                Action::DeleteBranch(_)
                | Action::RemoveRemote(_)
                | Action::AbortMerge
                | Action::DropStash(_),
                _,
            )
            | DialogKind::DeleteBranches => true,
            // These discard unsaved conflict-editor text or, for Exit, unsaved drafts.
            DialogKind::ConflictReload(_)
            | DialogKind::Confirm(Action::KeepConflictSide { .. }, _) => unsaved(dialog.owner),
            DialogKind::CloseTab(id) => unsaved(Some(*id)),
            DialogKind::Quit => {
                self.tabs.iter().any(|t| t.conflict.dirty) || self.state_error.is_some()
            }
            _ => false,
        }
    }
    pub(crate) fn menubar(&mut self, ui: &mut Ui, intents: &mut Vec<Intent>) {
        let opened = self.tab().is_some();
        let idle = self.write.is_none();
        ui.spacing_mut().interact_size.y = 24.0;
        MenuBar::new().ui(ui, |ui| {
            ui.spacing_mut().button_padding.x = 7.0;
            ui.spacing_mut().item_spacing.x = 9.0;
            let response = menu_title(ui, tr("File"), |ui| {
                let new = item_response(
                    ui,
                    tr("New Repository…"),
                    "Ctrl+N",
                    self.backend.is_some() && idle,
                );
                self.frame_widgets
                    .insert("repository-new".into(), (&new).into());
                if new.clicked() {
                    intents.push(Intent::Dialog(DialogKind::NewRepository));
                }
                let open = item_response(ui, tr("Open Repository…"), "Ctrl+O", self.backend.is_some());
                self.frame_widgets.insert("file-open".into(), (&open).into());
                if open.clicked() {
                    intents.push(Intent::Dialog(DialogKind::Open));
                }
                let clone = item_response(
                    ui,
                    tr("Clone Repository…"),
                    "Ctrl+Shift+O",
                    self.backend.is_some() && idle,
                );
                self.frame_widgets
                    .insert("file-clone".into(), (&clone).into());
                if clone.clicked() {
                    intents.push(Intent::Dialog(DialogKind::Clone));
                }
                let explore = item_response(ui, tr("Explore Repositories"), "Ctrl+Shift+E", self.backend.is_some());
                self.frame_widgets.insert("file-explore".into(), (&explore).into());
                if explore.clicked() {
                    intents.push(Intent::Explore(None));
                }
                let manager = item_response(ui, tr("Repository Manager…"), "Ctrl+Shift+M", self.backend.is_some() && idle);
                self.frame_widgets.insert("repository-manager".into(), (&manager).into());
                if manager.clicked() { intents.push(Intent::Dialog(DialogKind::RepositoryManager)); }
                submenu(ui, tr("Open Recent"), true, |ui| {
                    if self.recent.is_empty() {
                        item(ui, tr("No repositories this session"), "", false);
                    }
                    for path in &self.recent {
                        if item(ui, &crate::views::display_path(path), "", true) {
                            intents.push(Intent::Open(path.clone()));
                        }
                    }
                });
                ui.separator();
                if item(ui, tr("Close Repository"), "Ctrl+W", opened)
                    && let Some(tab) = self.tab()
                {
                    intents.push(Intent::CloseTab(tab.id));
                }
                if item(
                    ui,
                    tr("Close All Repositories"),
                    "",
                    opened && idle && self.tabs.iter().all(|t| t.draft.is_empty()),
                ) {
                    for tab in &self.tabs {
                        intents.push(Intent::CloseTab(tab.id));
                    }
                }
                // Accounts and SSH keys are in Preferences (Connections).
                ui.separator();
                if item(ui, tr("Exit"), "Alt+F4", true) {
                    intents.push(Intent::Exit);
                }
            });
            self.frame_widgets
                .insert("menu-file".into(), (&response.response).into());
            // Opening the Edit menu takes focus from the text field it acts on, so the
            // field focused when it opened is kept while the menu shows.
            if !menu_was_open(ui, tr("Edit")) {
                self.edit_target = focused_text_field(ui.ctx());
            }
            let response = menu_title(ui, tr("Edit"), |ui| self.edit_menu(ui, intents));
            self.frame_widgets
                .insert("menu-edit".into(), (&response.response).into());
            let view = menu_title(ui, tr("View"), |ui| {
                let palette = item_response(ui, tr("Command Palette…"), "Ctrl+K", true);
                self.frame_widgets
                    .insert("menu-palette".into(), (&palette).into());
                if palette.clicked() {
                    self.palette.show();
                }
                self.go_to_menu(ui, intents);
                ui.separator();
                if check_row(ui, tr("Show Sidebar"), self.show_sidebar, true).clicked() {
                    self.show_sidebar = !self.show_sidebar;
                }
                let response = check_row(ui, tr("Show Sidebar Labels"), self.sidebar_labels, true);
                self.frame_widgets
                    .insert("menu-sidebar-labels".into(), (&response).into());
                if response.clicked() {
                    let labels = !self.sidebar_labels;
                    let ctx = ui.ctx().clone();
                    // Same rules as the sidebar's own toggle, including narrow windows.
                    if labels == self.sidebar_rail(ctx.content_rect().width()) {
                        self.toggle_sidebar_labels(&ctx);
                    } else {
                        self.sidebar_labels = labels;
                    }
                }
                let response = check_row(ui, tr("Repositories in Sidebar"), self.repository_list, true);
                self.frame_widgets
                    .insert("menu-repository-list".into(), (&response).into());
                if response.clicked() {
                    self.repository_list = !self.repository_list;
                }
                if check_row(ui, tr("Show Commit Details"), self.show_details, true).clicked() {
                    self.show_details = !self.show_details;
                }
                // Unchecked while the drawer is collapsed: choosing it opens the drawer.
                let shown = self.show_console && !self.drawer_collapsed;
                if check_row(ui, tr("Show Activity"), shown, true).clicked() {
                    if shown {
                        self.show_console = false;
                    } else {
                        self.show_activity();
                    }
                }
                let history = item_response(ui, tr("Activity & Diagnostics…"), "", true);
                self.frame_widgets
                    .insert("activity-open-menu".into(), (&history).into());
                if history.clicked() {
                    self.activity_history.show();
                    self.external.open = false;
                }
                let benchmark = item_response(ui, tr("Repository Benchmark"), "", opened && !self.interaction_busy());
                self.frame_widgets.insert("benchmark-open".into(), (&benchmark).into());
                if benchmark.clicked() {
                    self.activity_history.close();
                    intents.push(Intent::Dialog(DialogKind::Benchmark));
                }
                ui.separator();
                for (collapse, label, key) in [
                    (true, tr("Collapse All Merges"), "view-collapse-merges"),
                    (false, tr("Expand All Merges"), "view-expand-merges"),
                ] {
                    let response = item_response(ui, label, "", opened);
                    self.frame_widgets.insert(key.into(), (&response).into());
                    if response.clicked() {
                        intents.push(Intent::FoldAllMerges(collapse));
                        ui.close();
                    }
                }
                submenu(ui, tr("Diff Layout"), opened, |ui| {
                    if let Some(tab) = self.tabs.get_mut(self.active) {
                        for (split, label) in [(false, tr("Unified")), (true, tr("Side by side"))] {
                            if check_row(ui, label, tab.split_diff == split, true).clicked() {
                                tab.split_diff = split;
                            }
                        }
                    }
                });
                if let Some(tab) = self.tabs.get_mut(self.active) {
                    ui.separator();
                    for (label, key, hidden) in [
                        (tr("Hide Tags in Sidebar"), "view-hide-tags", &mut tab.worktree_view.hide_tags),
                        (tr("Hide Stashes in Sidebar"), "view-hide-stashes", &mut tab.worktree_view.hide_stashes),
                    ] {
                        let r = check_row(ui, label, *hidden, true);
                        self.frame_widgets.insert(key.into(), (&r).into());
                        if r.clicked() { *hidden = !*hidden; }
                    }
                    let r = item_response(ui, tr("Reset This Worktree’s View"), "", true);
                    self.frame_widgets.insert("view-reset-worktree".into(), (&r).into());
                    if r.clicked() {
                        crate::worktree_view::Settings::default().restore(tab);
                        tab.relayout();
                        intents.push(Intent::History(None));
                    }
                }
                self.appearance_menu(ui);
                ui.separator();
                if item(ui, tr("Zoom In"), "Ctrl++", true) {
                    ui.ctx()
                        .set_zoom_factor((ui.ctx().zoom_factor() + 0.1).min(1.5));
                }
                if item(ui, tr("Zoom Out"), "Ctrl+−", true) {
                    ui.ctx()
                        .set_zoom_factor((ui.ctx().zoom_factor() - 0.1).max(0.75));
                }
                if item(ui, tr("Reset Zoom"), "Ctrl+0", true) {
                    ui.ctx().set_zoom_factor(1.0);
                }
                if item(ui, tr("Refresh Repository"), "F5", opened && idle) {
                    intents.push(Intent::Refresh);
                }
                // Compare itself is in Go to.
                let r = item_response(ui, tr("Compare Selected Commits"), "", self.tab().is_some_and(|t|t.selected_commits.len() == 2));
                pages::control(&mut self.frame_widgets, "menu-compare-selected", &r, ui.clip_rect());
                if r.clicked() { intents.push(Intent::Compare(crate::compare::CompareIntent::Selected)); }
            });
            self.frame_widgets
                .insert("menu-view".into(), (&view.response).into());
            let response = menu_title(ui, tr("Repository"), |ui| self.repository_menu(ui, intents));
            self.frame_widgets
                .insert("menu-repository".into(), (&response.response).into());
            let response = menu_title(ui, tr("Terminal"), |ui| {
                // The active repository's shell, the same actions as the drawer strip.
                let shell = self.terminal_state();
                let session = shell.is_some();
                let running = shell.is_some_and(|(running, _)| running) && !self.quitting;
                for (key, label, shortcut, enabled, action) in [
                    ("terminal-new", tr("New Terminal"), "Ctrl+Shift+`", opened, None),
                    ("terminal-console", tr("Show Activity"), "Ctrl+`", true, None),
                    ("terminal-interrupt-menu", tr("Interrupt"), "Ctrl+C", running, Some(TerminalAction::Interrupt)),
                    (
                        "terminal-clear-scrollback-menu",
                        tr("Clear scrollback"),
                        "",
                        session,
                        Some(TerminalAction::ClearScrollback),
                    ),
                    ("terminal-end-menu", tr("End session"), "", running, Some(TerminalAction::End)),
                ] {
                    if key == "terminal-interrupt-menu" {
                        ui.separator();
                    }
                    let response = item_response(ui, label, shortcut, enabled);
                    self.frame_widgets.insert(key.into(), (&response).into());
                    if response.clicked() {
                        match (key, action) {
                            (_, Some(action)) => self.terminal_action(action),
                            ("terminal-new", _) => intents.push(Intent::Terminal),
                            ("terminal-console", _) => self.show_activity(),
                            _ => {}
                        }
                    }
                }
            });
            self.frame_widgets
                .insert("menu-terminal".into(), (&response.response).into());
            let response = menu_title(ui, tr("Window"), |ui| {
                let next = item_response(ui, tr("Next Repository"), "Ctrl+Tab", self.tabs.len() > 1);
                self.frame_widgets.insert("window-next".into(), (&next).into());
                if next.clicked() {
                    self.active = (self.active + 1) % self.tabs.len();
                }
                let previous =
                    item_response(ui, tr("Previous Repository"), "Ctrl+Shift+Tab", self.tabs.len() > 1);
                self.frame_widgets
                    .insert("window-previous".into(), (&previous).into());
                if previous.clicked() {
                    self.active = (self.active + self.tabs.len() - 1) % self.tabs.len();
                }
                // MCP server settings are in Preferences (and the status-bar MCP chip).
                if !self.tabs.is_empty() {
                    ui.separator();
                    menu_caption(ui, tr("OPEN REPOSITORIES"));
                }
                for (i, tab) in self.tabs.iter().enumerate() {
                    if check_row(ui, &tab.title, i == self.active, true).clicked() {
                        self.active = i;
                    }
                }
            });
            self.frame_widgets
                .insert("menu-window".into(), (&response.response).into());
            let response = menu_title(ui, tr("Help"), |ui| {
                let setup = item_response(ui, tr("Setup Guide"), "", self.write.is_none());
                self.frame_widgets.insert("help-setup".into(), (&setup).into());
                if setup.clicked() { intents.push(Intent::GuidedSetup); }
                let r = item_response(ui, tr("Guide"), "F1", true);
                self.frame_widgets.insert("help-guide".into(), (&r).into());
                if r.clicked() {
                    crate::guide::request(ui.ctx());
                }
                ui.separator();
                let r = item_response(ui, tr("View Release Notes…"), "", true);
                self.frame_widgets
                    .insert("help-changelog".into(), (&r).into());
                if r.clicked() {
                    intents.push(Intent::Dialog(DialogKind::Changelog));
                }
                let r=item_response(ui,tr("Check for Updates"),"",true);
                self.frame_widgets.insert("help-updates".into(),(&r).into());
                if r.clicked(){self.updates.open();}
                let r=item_response(ui,tr("Crash Reports"),"",true);
                self.frame_widgets.insert("help-crash-reports".into(),(&r).into());
                if r.clicked(){self.crash_reports.open=true;self.crash_reports.refresh_requested=true;}
                ui.separator();
                let r = item_response(ui, tr("About"), "", true);
                self.frame_widgets.insert("help-about".into(), (&r).into());
                if r.clicked() {
                    intents.push(Intent::Dialog(DialogKind::About));
                }
            });
            self.frame_widgets
                .insert("menu-help".into(), (&response.response).into());
        });
    }
    /// Edit: the text commands act on the field the menu was opened from, then Undo
    /// Last Commit, which only ever undoes a commit.
    fn edit_menu(&mut self, ui: &mut Ui, intents: &mut Vec<Intent>) {
        let target = self.edit_target;
        let selected = target.and_then(|t| t.selection).is_some_and(|range| !range.is_empty());
        for (key, label, shortcut, command, enabled) in [
            ("edit-undo", tr("Undo"), "Ctrl+Z", TextCommand::Undo, target.is_some()),
            ("edit-redo", tr("Redo"), "Ctrl+Y", TextCommand::Redo, target.is_some()),
            ("edit-cut", tr("Cut"), "Ctrl+X", TextCommand::Cut, selected),
            ("edit-copy", tr("Copy"), "Ctrl+C", TextCommand::Copy, selected),
            ("edit-paste", tr("Paste"), "Ctrl+V", TextCommand::Paste, target.is_some()),
            ("edit-select-all", tr("Select All"), "Ctrl+A", TextCommand::SelectAll, target.is_some()),
        ] {
            if command == TextCommand::Cut {
                ui.separator();
            }
            let response = item_response(ui, label, shortcut, enabled);
            self.frame_widgets.insert(key.into(), (&response).into());
            if response.clicked()
                && let Some(target) = target
            {
                if command == TextCommand::Paste {
                    // The clipboard's text arrives as a paste event next frame.
                    ui.ctx().send_viewport_cmd(ViewportCommand::RequestPaste);
                }
                self.text_command = Some((target, command));
                ui.ctx().request_repaint();
            }
        }
        ui.separator();
        let undo = item_response(ui, tr("Undo Last Commit"), "", self.write.is_none() && self.tab().is_some_and(|t| t.undo_commit.is_some() && !t.loading()));
        self.frame_widgets.insert("edit-undo-commit".into(), (&undo).into());
        if undo.clicked() {
            intents.push(Intent::Dialog(DialogKind::UndoCommit));
        }
        ui.separator();
        let response = item_response(ui, tr("Preferences…"), "Ctrl+,", true);
        self.frame_widgets
            .insert("preferences".into(), (&response).into());
        if response.clicked() {
            intents.push(Intent::Dialog(DialogKind::Preferences));
        }
    }
    /// Runs the Edit menu command chosen last frame on its text field. The click that
    /// chose it took the field's focus, and a click would take it again, so the field
    /// gets focus, its selection and its event a frame later. Called before anything
    /// reads the frame's input, so pasted text reaches the field instead of being taken
    /// for a patch.
    pub(crate) fn apply_text_command(&mut self, ctx: &Context) {
        let Some((target, command)) = self.text_command.take() else {
            return;
        };
        if let Some(mut state) = TextEdit::load_state(ctx, target.field) {
            state.cursor.set_char_range(target.selection);
            TextEdit::store_state(ctx, target.field, state);
        }
        ctx.memory_mut(|m| m.request_focus(target.field));
        if let Some(event) = command.event() {
            ctx.input_mut(|i| i.events.push(event));
        }
    }
    /// View → Go to: the sidebar's pages in its order, the current one ticked, for when
    /// the sidebar is hidden or shows icons only. Same rules as the sidebar: views open
    /// any time; pages wait while a write or a foreground load runs.
    fn go_to_menu(&mut self, ui: &mut Ui, intents: &mut Vec<Intent>) {
        use crate::pages::Page;
        enum To {
            View(View),
            Page(Page),
            Line,
        }
        let opened = self.tab().is_some();
        let free = !self.interaction_busy();
        let backend = self.backend.is_some();
        let page = self.page();
        let current = self.tab().filter(|_| page.is_none()).map(|t| t.view);
        let entries = self.integrations.entries().to_vec();
        let open = self.integrations.open_entry().filter(|_| page == Some(Page::Integrations)).map(|e| e.id.clone());
        keyed_submenu(ui, &mut self.frame_widgets, "view-goto", tr("Go to"), true, |ui, widgets| {
            for (to, label, key) in [
                (To::View(View::Summary), tr("Summary"), "goto-summary"),
                (To::View(View::Changes), tr("Changes"), "goto-changes"),
                (To::View(View::Commits), tr("Commits"), "goto-commits"),
                (To::View(View::Compare), tr("Compare"), "goto-compare"),
                (To::View(View::Map), tr("Repository map"), "goto-map"),
                (To::View(View::BranchReview), tr("AI review"), "goto-review"),
                (To::View(View::Actions), tr("Actions"), "goto-actions"),
                (To::View(View::Agents), tr("Agents"), "goto-agents"),
                (To::View(View::Tree), tr("Files"), "goto-files"),
                (To::Line, "", ""),
                (To::View(View::Branches), tr("Branches"), "goto-branches"),
                (To::View(View::Remotes), tr("Remotes"), "goto-remotes"),
                (To::View(View::Stashes), tr("Stashes"), "goto-stashes"),
                (To::Line, "", ""),
                (To::Page(Page::Tags), tr("Tags"), "goto-tags"),
                (To::Page(Page::Worktrees), tr("Worktrees"), "goto-worktrees"),
                (To::Page(Page::Submodules), tr("Submodules"), "goto-submodules"),
                (To::Page(Page::Lfs), tr("Git LFS"), "goto-lfs"),
                (To::View(View::FileHistory), tr("Path History"), "goto-history"),
                (To::Page(Page::Statistics), tr("Statistics"), "goto-statistics"),
                (To::Page(Page::Benchmark), tr("Repository Benchmark"), "goto-benchmark"),
            ] {
                let (checked, enabled, intent) = match to {
                    To::Line => {
                        ui.separator();
                        continue;
                    }
                    To::View(view) => (current == Some(view), opened, Intent::View(view)),
                    To::Page(p) => (page == Some(p), opened && free, Intent::Dialog(p.dialog_kind())),
                };
                let response = check_row(ui, label, checked, enabled);
                pages::control(widgets, key, &response, ui.clip_rect());
                if response.clicked() {
                    intents.push(intent);
                }
            }
            // Extensions need no repository.
            ui.separator();
            for entry in entries {
                let response = check_row(ui, &entry.name, open.as_ref() == Some(&entry.id), backend && free);
                pages::control(widgets, format!("goto-ext-{}", entry.id), &response, ui.clip_rect());
                if response.clicked() {
                    intents.push(Intent::Extension(entry.id));
                }
            }
            let response = check_row(ui, tr("Extensions"), false, backend);
            pages::control(widgets, "goto-integrations", &response, ui.clip_rect());
            if response.clicked() {
                intents.push(Intent::Preferences(crate::preferences::Page::Integrations));
            }
        });
    }
    /// Repository: Fetch, Pull and Push on top, everything else in submenus by task.
    /// Items keep the ids they had in the flat menu; automation opens the submenu first.
    fn repository_menu(&mut self, ui: &mut Ui, intents: &mut Vec<Intent>) {
        use crate::shortcuts::model::Action as Shortcut;
        use rocket_git::{CommitOperation as Operation, RebaseAction};
        let writable = self.can_write();
        let opened = self.tab().is_some();
        let idle = self.write.is_none();
        let busy = self.busy();
        let interaction_busy = self.interaction_busy();
        let state = self.tab().and_then(|t| t.snapshot.as_ref()).and_then(|s| s.state.as_ref());
        // Rebase starts only from a settled repository; commit actions only while no
        // other operation is under way.
        let settled = state.is_some_and(|s| s.is_idle());
        let operating = state.is_some_and(|s| !s.is_idle());
        let picks = self.tab().map_or_else(Vec::new, |t| t.commit_targets(Operation::CherryPick));
        let reverts = self.tab().map_or_else(Vec::new, |t| t.commit_targets(Operation::Revert));
        let [fetch, pull, push] = [Shortcut::Fetch, Shortcut::Pull, Shortcut::Push].map(|a| self.keybindings.bindings.label(a));
        let height = (ui.ctx().content_rect().height() - 80.0).max(180.0);
        let commands = &self.commands;
        let widgets = &mut self.frame_widgets;
        // egui sizes a new popup within its 400-point default area and keeps that size;
        // without a minimum the menu stayed ~386 points tall at every window size. It
        // still shrinks to its rows and scrolls in short windows.
        let body = ScrollArea::vertical()
            .id_salt("repository-menu-scroll")
            .max_height(height)
            .min_scrolled_height(height)
            .show(ui, |ui| {
                if hinted_item(ui, tr("Fetch…"), &fetch, opened && idle).clicked() {
                    intents.push(Intent::Dialog(DialogKind::Fetch));
                }
                if hinted_item(ui, tr("Pull…"), &pull, writable).clicked() {
                    intents.push(Intent::Dialog(DialogKind::Pull));
                }
                if hinted_item(ui, tr("Push…"), &push, writable).clicked() {
                    intents.push(Intent::Dialog(DialogKind::Push));
                }
                if keyed_item(ui, widgets, "repository-atomic-push", tr("Push Branches Together…"), "", writable) {
                    intents.push(Intent::Dialog(DialogKind::AtomicPush));
                }
                ui.separator();
                keyed_submenu(ui, widgets, "repository-commit-menu", tr("Commit"), opened, |ui, widgets| {
                    if item(ui, tr("Commit…"), "Ctrl+Enter", writable) {
                        intents.push(Intent::View(View::Changes));
                    }
                    for (operation, key, targets) in [
                        (Operation::Amend, "commit-amend", Vec::new()),
                        (Operation::CherryPick, "commit-cherry-pick", picks),
                        (Operation::Revert, "commit-revert", reverts),
                    ] {
                        if operation == Operation::CherryPick {
                            ui.separator();
                        }
                        let label = match (operation, targets.len()) {
                            (Operation::Amend, _) => tr("Amend Latest Commit…").to_owned(),
                            (Operation::CherryPick, 0 | 1) => tr("Cherry-pick Selected Commit…").into(),
                            (Operation::CherryPick, n) => trf!("Cherry-pick {n} Selected Commits…", n),
                            (_, 0 | 1) => tr("Revert Selected Commit…").into(),
                            (_, n) => trf!("Revert {n} Selected Commits…", n),
                        };
                        let ready = operation == Operation::Amend || !targets.is_empty();
                        if keyed_item(ui, widgets, key, &label, "", writable && !operating && ready) {
                            intents.push(Intent::Dialog(DialogKind::CommitAction(
                                rocket_git::CommitActionRequest { operation, targets, mainline: None, recovery: None },
                            )));
                        }
                    }
                });
                keyed_submenu(ui, widgets, "repository-branch-menu", tr("Branch"), opened, |ui, widgets| {
                    if item(ui, tr("New Branch…"), "Ctrl+Shift+B", writable) {
                        intents.push(Intent::Dialog(DialogKind::NewBranch));
                    }
                    ui.separator();
                    if keyed_item(ui, widgets, "repository-merge", tr("Merge Branch…"), "", writable) {
                        intents.push(Intent::Dialog(DialogKind::Merge));
                    }
                    for (key, label, action) in [
                        ("repository-rebase", tr("Rebase…"), RebaseAction::Start),
                        ("repository-interactive-rebase", tr("Interactive Rebase…"), RebaseAction::Interactive),
                    ] {
                        if keyed_item(ui, widgets, key, label, "", writable && settled) {
                            intents.push(Intent::Dialog(DialogKind::Rebase(action)));
                        }
                    }
                    ui.separator();
                    if keyed_item(ui, widgets, "repository-delete-branches", tr("Delete Branches…"), "", writable) {
                        intents.push(Intent::Dialog(DialogKind::DeleteBranches));
                    }
                    if keyed_item(ui, widgets, "repository-workflows", tr("Workflows & Cleanup…"), "", writable) {
                        intents.push(Intent::Dialog(DialogKind::Workflows));
                    }
                });
                keyed_submenu(ui, widgets, "repository-gitflow", tr("Git Flow"), opened, |ui, widgets| {
                    use crate::gitflow::Request;
                    use rocket_git::GitFlowKind as Kind;
                    for (label, key, request) in [
                        (tr("Initialize…"), "gitflow-init", Request::Init),
                        (tr("Start Feature…"), "gitflow-start-feature", Request::Start(Kind::Feature)),
                        (tr("Start Release…"), "gitflow-start-release", Request::Start(Kind::Release)),
                        (tr("Start Hotfix…"), "gitflow-start-hotfix", Request::Start(Kind::Hotfix)),
                        (tr("Start Support…"), "gitflow-start-support", Request::Start(Kind::Support)),
                        (tr("Finish…"), "gitflow-finish", Request::Finish),
                    ] {
                        if matches!(key, "gitflow-start-feature" | "gitflow-finish") {
                            ui.separator();
                        }
                        if keyed_item(ui, widgets, key, label, "", writable) {
                            intents.push(Intent::Dialog(DialogKind::GitFlow(request)));
                        }
                    }
                });
                keyed_submenu(ui, widgets, "repository-stash-menu", tr("Stash"), opened, |ui, _| {
                    if item(ui, tr("Save Changes…"), "", writable) {
                        intents.push(Intent::Dialog(DialogKind::Stash));
                    }
                    if item(ui, tr("Save Snapshot (Keep Local Changes)…"), "", writable) {
                        intents.push(Intent::KeepLocal { snapshot: true, paths: Vec::new() });
                    }
                    ui.separator();
                    if item(ui, tr("View Stashes"), "", opened) {
                        intents.push(Intent::View(View::Stashes));
                    }
                });
                keyed_submenu(ui, widgets, "repository-patches-menu", tr("Patches"), opened, |ui, widgets| {
                    use crate::patches::Mode;
                    if keyed_item(ui, widgets, "repository-patch-import", tr("Import Patch…"), "", opened && !busy) {
                        intents.push(Intent::Dialog(DialogKind::Patch(Mode::Import)));
                    }
                    if keyed_item(ui, widgets, "repository-series-import", tr("Import Commit Series…"), "", writable && !busy) {
                        intents.push(Intent::Dialog(DialogKind::MailImport(rocket_git::MailImportAction::Start)));
                    }
                    if keyed_item(ui, widgets, "repository-patch-clipboard", tr("Apply Patch from Clipboard…"), "Ctrl+V", opened && !busy) {
                        intents.push(Intent::PastePatch);
                    }
                    ui.separator();
                    if keyed_item(ui, widgets, "repository-patch-export", tr("Export Patch…"), "", opened && !busy) {
                        intents.push(Intent::Dialog(DialogKind::Patch(Mode::Export)));
                    }
                    if keyed_item(ui, widgets, "repository-series-export", tr("Export Commit Series…"), "", opened && !busy) {
                        intents.push(Intent::ExportSeries(None));
                    }
                });
                ui.separator();
                keyed_submenu(ui, widgets, "repository-history-menu", tr("History & Recovery"), opened, |ui, widgets| {
                    if keyed_item(ui, widgets, "repository-recovery", tr("Reflog & Recovery…"), "", opened && !busy) {
                        intents.push(Intent::Dialog(DialogKind::Recovery));
                    }
                    if keyed_item(ui, widgets, "repository-file-backups", tr("File Backups & Restore…"), "", opened && !busy) {
                        intents.push(Intent::Dialog(DialogKind::FileEdit(None)));
                    }
                    ui.separator();
                    if keyed_item(ui, widgets, "repository-tree", tr("Browse Files at Commit…"), "", opened && !interaction_busy) {
                        intents.push(Intent::Tree(crate::tree::TreeIntent::OpenSelected));
                    }
                    if keyed_item(ui, widgets, "repository-search-files", tr("Search in Files"), "", opened && !interaction_busy) {
                        intents.push(Intent::Tree(crate::tree::TreeIntent::FocusSearch));
                    }
                    if keyed_item(ui, widgets, "repository-bisect", tr("Bisect…"), "", opened && !interaction_busy) {
                        intents.push(Intent::Dialog(DialogKind::Bisect));
                    }
                });
                keyed_submenu(ui, widgets, "repository-custom-commands", tr("Custom Commands"), opened, |ui, widgets| {
                    crate::custom_commands::menu_entries(ui, commands, rocket_git::CommandScope::Repository, None, writable, intents, widgets);
                    if !commands.is_empty() {
                        ui.separator();
                    }
                    if keyed_item(ui, widgets, "command-edit", tr("Edit Custom Commands…"), "", idle) {
                        intents.push(Intent::Preferences(crate::preferences::Page::Commands));
                    }
                });
                extensions_menu(ui, widgets, opened, intents);
                ui.separator();
                keyed_submenu(ui, widgets, "repository-remotes-menu", tr("Remotes & Hosting"), opened, |ui, widgets| {
                    if keyed_item(ui, widgets, "repository-manage-remotes", tr("Manage Remotes…"), "", opened) {
                        intents.push(Intent::View(View::Remotes));
                    }
                    ui.separator();
                    if keyed_item(ui, widgets, "repository-hosting", tr("Hosting"), "", opened) {
                        intents.push(Intent::Dialog(DialogKind::Hosting));
                    }
                    if keyed_item(ui, widgets, "repository-github", tr("GitHub Repositories & Notifications…"), "", opened) {
                        intents.push(Intent::Dialog(DialogKind::GitHub));
                    }
                });
                keyed_submenu(ui, widgets, "repository-settings-menu", tr("Settings"), opened, |ui, widgets| {
                    if keyed_item(ui, widgets, "repository-settings", tr("Repository Settings…"), "", opened && idle) {
                        intents.push(Intent::Dialog(DialogKind::RepositorySettings));
                    }
                    if keyed_item(ui, widgets, "repository-signing", tr("Signing & Verification…"), "", opened && idle) {
                        intents.push(Intent::Dialog(DialogKind::Signing(None)));
                    }
                    if keyed_item(ui, widgets, "repository-ignore", tr("Ignored Files (.gitignore)…"), "", writable) {
                        intents.push(Intent::Dialog(DialogKind::Ignore(Vec::new())));
                    }
                    if keyed_item(ui, widgets, "repository-dependencies", tr("Dependency Security…"), "", opened) {
                        intents.push(Intent::Preferences(crate::preferences::Page::Security));
                    }
                });
            });
        // `inner_rect` is the size before the area shrinks to its rows.
        let shown = Rect::from_min_size(body.inner_rect.min, body.inner_rect.size().min(body.content_size));
        widgets.insert(
            "repository-menu-scroll".into(),
            UiControl {
                rect: shown.intersect(ui.clip_rect()),
                enabled: true,
            },
        );
    }
    pub(crate) fn shortcuts(&mut self, ctx: &Context, intents: &mut Vec<Intent>) {
        self.run_shortcuts(ctx, intents);
    }
    pub(crate) fn dialog_ui(&mut self, ctx: &Context) {
        if self.dialog.as_ref().is_some_and(|d| matches!(d.kind, DialogKind::UndoCommit)) {
            self.undo_dialog(ctx); return;
        }
        if self.dialog.as_ref().is_some_and(|d| matches!(d.kind, DialogKind::RepositoryManager)) {
            self.repository_manager_dialog(ctx); return;
        }
        if self.dialog.as_ref().is_some_and(|d| matches!(d.kind, DialogKind::Open)) {
            self.open_dialog(ctx);
            return;
        }
        if self.dialog.as_ref().is_some_and(|d| matches!(d.kind, DialogKind::Preferences)) {
            self.preferences_dialog(ctx);
            return;
        }
        if self
            .dialog
            .as_ref()
            .is_some_and(|d| matches!(d.kind, DialogKind::Clone | DialogKind::NewRepository | DialogKind::KeepPreview(_)))
        {
            self.repository_form_dialog(ctx);
            return;
        }
        // Explore is a page; Preview opens a tab and nothing here is modal.
        if self.dialog.as_ref().is_some_and(|d| matches!(d.kind, DialogKind::Explore)) {
            return;
        }
        self.drop_menu(ctx);
        if self.dialog.as_ref().is_some_and(|d|matches!(d.kind,DialogKind::AtomicPush)){self.atomic_push_dialog(ctx);return;}
        if self.dialog.as_ref().is_some_and(|d|matches!(d.kind,DialogKind::Ignore(_))){self.ignore_dialog(ctx);return;}
        if self.dialog.as_ref().is_some_and(|d|matches!(d.kind,DialogKind::Workflows)){self.workflow_dialog(ctx);return;}
        if self
            .dialog
            .as_ref()
            .is_some_and(|d| matches!(d.kind, DialogKind::Push) && d.force_push.is_some())
        {
            self.force_push_dialog(ctx);
            return;
        }
        if self
            .dialog
            .as_ref()
            .is_some_and(|d| matches!(d.kind, DialogKind::Pull))
        {
            self.pull_dialog(ctx);
            return;
        }
        if self
            .dialog
            .as_ref()
            .is_some_and(|d| matches!(d.kind, DialogKind::Bisect))
        {
            self.bisect_dialog(ctx);
            return;
        }
        if self
            .dialog
            .as_ref()
            .is_some_and(|d| matches!(d.kind, DialogKind::Merge))
        {
            self.merge_dialog(ctx);
            return;
        }
        if self
            .dialog
            .as_ref()
            .is_some_and(|d| matches!(d.kind, DialogKind::GitFlow(_)))
        {
            self.gitflow_dialog(ctx);
            return;
        }
        if self
            .dialog
            .as_ref()
            .is_some_and(|d| matches!(d.kind, DialogKind::DraftNotice(_)))
        {
            self.draft_notice_dialog(ctx);
            return;
        }
        if self
            .dialog
            .as_ref()
            .is_some_and(|d| matches!(d.kind, DialogKind::Patch(_)))
        {
            self.patch_dialog(ctx);
            return;
        }
        if self
            .dialog
            .as_ref()
            .is_some_and(|d| matches!(d.kind, DialogKind::FileEdit(_)))
        {
            self.file_edit_dialog(ctx);
            return;
        }
        if self
            .dialog
            .as_ref()
            .is_some_and(|d| matches!(d.kind, DialogKind::Worktrees))
        {
            self.worktree_dialog(ctx);
            return;
        }
        if self
            .dialog
            .as_ref()
            .is_some_and(|d| matches!(d.kind, DialogKind::Lfs))
        {
            self.lfs_dialog(ctx);
            return;
        }
        if self
            .dialog
            .as_ref()
            .is_some_and(|d| matches!(d.kind, DialogKind::Submodules))
        {
            self.submodule_dialog(ctx);
            return;
        }
        if self
            .dialog
            .as_ref()
            .is_some_and(|d| matches!(d.kind, DialogKind::Tags))
        {
            self.tag_dialog(ctx);
            return;
        }
        if self
            .dialog
            .as_ref()
            .is_some_and(|d| matches!(d.kind, DialogKind::GitHub))
        {
            self.github_dialog();
            return;
        }
        if self.dialog.as_ref().is_some_and(|d| matches!(d.kind, DialogKind::Hosting | DialogKind::Statistics | DialogKind::Benchmark | DialogKind::Integrations)) { return; }
        if self
            .dialog
            .as_ref()
            .is_some_and(|d| matches!(d.kind, DialogKind::Recovery))
        {
            self.recovery_dialog(ctx);
            return;
        }
        if self
            .dialog
            .as_ref()
            .is_some_and(|d| matches!(d.kind, DialogKind::Rebase(_)))
        {
            self.rebase_dialog(ctx);
            return;
        }
        if self.dialog.as_ref().is_some_and(|d|matches!(d.kind,DialogKind::MailImport(_))) { self.mail_import_dialog(ctx); return; }
        if self
            .dialog
            .as_ref()
            .is_some_and(|d| matches!(d.kind, DialogKind::CommitAction(_)))
        {
            self.commit_action_dialog(ctx);
            return;
        }
        if self
            .dialog
            .as_ref()
            .is_some_and(|d| matches!(d.kind, DialogKind::Signing(_)))
        {
            self.signing_dialog(ctx);
            return;
        }
        if self.dialog.as_ref().is_some_and(|d| {
            matches!(d.kind, DialogKind::About | DialogKind::Legal | DialogKind::Changelog)
        }) {
            self.app_dialog(ctx);
            return;
        }
        let Some(mut dialog) = self.dialog.take() else {
            return;
        };
        let mut submit = false;
        let mut command_browse = None;
        let mut dismiss = false;
        let informational = matches!(
            dialog.kind,
            DialogKind::Preferences | DialogKind::About | DialogKind::Legal | DialogKind::Changelog
        );
        let (image, mut title, mut subtitle) = dialog_header(&dialog);
        if let DialogKind::RunCommand(index) = dialog.kind
            && let Some(command) = self.commands.get(index)
        {
            let name = rocket_git::custom_command_display(command.name.trim());
            title = if command.action==rocket_git::CommandAction::Url {trf!("Open {command}", command = name)} else {trf!("Run {command}", command = name)};
            if let Some(tab) = self.tabs.iter().find(|t| Some(t.id) == dialog.owner) {
                subtitle = if command.action==rocket_git::CommandAction::Url {trf!("From {repository}. Review the full address before opening your browser.", repository = tab.title)} else {trf!("In {repository}, without a shell. Output goes to Activity.", repository = tab.title)};
            }
        }
        if let DialogKind::CloseTab(id) = dialog.kind
            && let Some(tab) = self.tabs.iter().find(|t| t.id == id)
        {
            title = trf!("Close {repository}?", repository = tab.title);
            subtitle = crate::views::display_path(&tab.path);
        }
        let danger = self.dialog_danger(&dialog);
        let head = pages::ReviewHeader {
            image,
            title: &title,
            subtitle: &subtitle,
            danger,
        };
        let quit_rows = if matches!(dialog.kind, DialogKind::Quit) {
            self.quit_rows()
        } else {
            Vec::new()
        };
        let mut content =
            |ui: &mut Ui, part: pages::Part, widgets: &mut std::collections::HashMap<String, UiControl>| match part {
                pages::Part::Body => {
                    match &dialog.kind {
                        DialogKind::RepositoryManager | DialogKind::UndoCommit | DialogKind::Explore | DialogKind::KeepPreview(_) => unreachable!("separate manager review"),
                        DialogKind::Signing(_)
                        | DialogKind::CommitAction(_)
                        | DialogKind::Rebase(_)
                        | DialogKind::MailImport(_)
                        | DialogKind::Bisect
                        | DialogKind::Recovery
                        | DialogKind::Tags
                        | DialogKind::Worktrees
                        | DialogKind::Submodules
                        | DialogKind::Lfs
                        | DialogKind::GitHub
        | DialogKind::Hosting
        | DialogKind::Integrations
        | DialogKind::Statistics
        | DialogKind::Benchmark
                        | DialogKind::FileEdit(_)
                        | DialogKind::GitFlow(_)
                        | DialogKind::DraftNotice(_)
                        | DialogKind::Patch(_) => {
                            unreachable!("separate reviewed operation dialog")
                        }
                        DialogKind::RepositorySettings
                            if dialog.issue_links.as_ref().is_some_and(|e| e.shown) =>
                        {
                            crate::issue_links::pages(ui, &mut dialog.issue_links, widgets);
                            if let Some(editor) = &mut dialog.issue_links {
                                editor.ui(ui, widgets);
                            }
                        }
                        DialogKind::RepositorySettings => {
                            crate::issue_links::pages(ui, &mut dialog.issue_links, widgets);
                            if dialog.identity_task.is_some() {
                                pages::busy_line(ui, tr("Loading Git identity settings…"));
                            }
                            crate::small_dialogs::identity_body(ui, &mut dialog, widgets);
                        }
                        DialogKind::Open
                        | DialogKind::Preferences
                        | DialogKind::Clone
                        | DialogKind::NewRepository => {
                            unreachable!("dedicated dialog")
                        }
                        DialogKind::NewBranch | DialogKind::TrackBranch { .. } | DialogKind::Rename(_) => {
                            let tab = self.tabs.iter().find(|t| Some(t.id) == dialog.owner);
                            crate::small_dialogs::branch_name_body(ui, &mut dialog, tab, widgets);
                        }
                        DialogKind::AddRemote => {
                            let tab = self.tabs.iter().find(|t| Some(t.id) == dialog.owner);
                            crate::small_dialogs::add_remote_body(ui, &mut dialog, tab, self.backend.as_ref(), widgets);
                        }
                        DialogKind::EditRemote(..) => {
                            let tab = self.tabs.iter().find(|t| Some(t.id) == dialog.owner);
                            crate::small_dialogs::edit_remote_body(ui, &mut dialog, tab, self.backend.as_ref(), widgets);
                        }
                        DialogKind::Fetch => {
                            let tab = self.tabs.iter().find(|t| Some(t.id) == dialog.owner);
                            crate::small_dialogs::fetch_body(ui, &mut dialog, tab, widgets);
                        }
                        DialogKind::Push => {
                            let tab = self.tabs.iter().find(|t| Some(t.id) == dialog.owner);
                            crate::small_dialogs::push_body(ui, &mut dialog, tab, self.backend.as_ref(), widgets);
                        }
                        DialogKind::Pull => {} // Dedicated fetch/review/apply dialog above.
                        DialogKind::Stash => {
                            let tab = self.tabs.iter().find(|t| Some(t.id) == dialog.owner);
                            crate::small_dialogs::stash_body(ui, &mut dialog, tab, widgets);
                        }
                        DialogKind::Merge => unreachable!("dedicated merge review"),
                        DialogKind::DeleteBranches => {
                            let tab = self.tabs.iter().find(|t| Some(t.id) == dialog.owner);
                            crate::small_dialogs::delete_branches_body(ui, &mut dialog, tab, self.backend.as_ref(), widgets);
                        }
                        DialogKind::RunCommand(index) => {
                            let branches = self.tabs.iter().find(|t| Some(t.id) == dialog.owner)
                                .and_then(|t| t.snapshot.as_ref()).map(|s| s.branches.iter()
                                .map(|b| short_ref(&b.reference)).collect::<Vec<_>>()).unwrap_or_default();
                            if let Some(command) = self.commands.get(*index) {
                                command_browse = crate::custom_commands::form(ui, command, &mut dialog.command_values, &branches, widgets);
                            }
                            let context = self
                                .tabs
                                .iter()
                                .find(|t| Some(t.id) == dialog.owner)
                                .and_then(|t| crate::custom_commands::selected_context(t, dialog.command_context.as_ref()));
                            crate::custom_commands::confirmation(
                                ui,
                                self.commands.get(*index),
                                context.as_ref(),
                                &dialog.command_values,
                            );
                        }
                        DialogKind::Confirm(
                            Action::DeleteBranch(_)
                            | Action::RemoveRemote(_)
                            | Action::DropStash(_)
                            | Action::ApplyStash { .. },
                            _,
                        ) => {
                            let tab = self.tabs.iter().find(|t| Some(t.id) == dialog.owner);
                            crate::small_dialogs::confirm_body(ui, &mut dialog, tab, self.backend.as_ref(), widgets);
                        }
                        // The save message is several paragraphs, too long for the header.
                        DialogKind::Confirm(
                            Action::SaveConflict { .. } | Action::KeepConflictSide { .. },
                            message,
                        ) => {
                            ui.add(Label::new(message).wrap());
                        }
                        DialogKind::Confirm(..) => {}
                        DialogKind::ConflictReload(_) => {
                            pages::callout(ui, AMBER(), tr("This replaces your editor draft."));
                        }
                        DialogKind::CloseTab(id) => {
                            let tab = self.tabs.iter().find(|t| t.id == *id);
                            crate::small_dialogs::close_body(ui, tab);
                            if let Some(error) = &self.state_error {
                                pages::callout(ui, AMBER(), error);
                            }
                        }
                        DialogKind::Quit => crate::small_dialogs::quit_body(ui, &quit_rows),
                        DialogKind::Changelog | DialogKind::About | DialogKind::Legal => {
                            unreachable!("app dialog")
                        }
                        DialogKind::AtomicPush | DialogKind::Ignore(_) | DialogKind::Workflows => unreachable!("Dedicated review dialog"),
                    }
                    if let Some(error) = &dialog.error {
                        pages::callout(ui, RED(), error);
                    }
                }
                pages::Part::Footer => {
                    let tab = self.tabs.iter().find(|t| Some(t.id) == dialog.owner);
                    let ready = match &dialog.kind {
                        DialogKind::RepositorySettings
                            if dialog.issue_links.as_ref().is_some_and(|e| e.shown) =>
                        {
                            self.write.is_none()
                                && dialog.issue_links.as_ref().is_some_and(|e| {
                                    e.settings.is_some()
                                        && !e.running()
                                        && e.changed()
                                        && e.problem().is_none()
                                })
                        }
                        DialogKind::RepositorySettings => {
                            dialog.identity.is_some()
                                && dialog.identity_task.is_none()
                                && self.write.is_none()
                                && crate::small_dialogs::identity_ready(&dialog)
                        }
                        DialogKind::NewBranch | DialogKind::TrackBranch { .. } | DialogKind::Rename(_) => {
                            crate::small_dialogs::branch_name_ready(&dialog, tab)
                        }
                        DialogKind::Open | DialogKind::Merge => !dialog.first.trim().is_empty(),
                        DialogKind::AddRemote => {
                            crate::small_dialogs::add_remote_ready(&dialog, tab) && self.write.is_none()
                        }
                        DialogKind::EditRemote(_, kind) => {
                            self.write.is_none()
                                && (!dialog.first.trim().is_empty()
                                    || (*kind == rocket_git::RemoteUrlKind::Push && dialog.flag))
                                && !crate::small_dialogs::edit_remote_unchanged(&dialog)
                        }
                        DialogKind::Fetch => !dialog.remotes.is_empty() && self.write.is_none(),
                        DialogKind::DeleteBranches => {
                            !dialog.selected_branches.is_empty() && self.write.is_none()
                        }
                        DialogKind::RunCommand(index) => {
                            self.write.is_none()
                                && (crate::custom_commands::command_line(
                                    &self.commands,
                                    &self.tabs,
                                    dialog.owner,
                                    *index,
                                    &dialog.command_values,
                                    dialog.command_context.as_ref(),
                                )
                                .is_some() || crate::custom_commands::command_url(&self.commands,&self.tabs,dialog.owner,*index,&dialog.command_values,dialog.command_context.as_ref()).is_some())
                        }
                        // Something to save; a partial stash needs at least one file.
                        DialogKind::Stash => {
                            crate::small_dialogs::stash_ready(&dialog, tab) && self.write.is_none()
                        }
                        DialogKind::Push => {
                            !dialog.remotes.is_empty()
                                && !dialog.branch.is_empty()
                                && !dialog.second.trim().is_empty()
                                && self.write.is_none()
                        }
                        DialogKind::Preferences
                        | DialogKind::About
                        | DialogKind::Legal
                        | DialogKind::Changelog
                        | DialogKind::CloseTab(_)
                        | DialogKind::Quit => true,
                        _ => self.write.is_none(),
                    };
                    let label = match dialog.kind {
                        DialogKind::Open => tr("Open"),
                        DialogKind::RepositorySettings
                            if dialog.issue_links.as_ref().is_some_and(|e| e.shown) =>
                        {
                            tr("Save issue links")
                        }
                        DialogKind::RepositorySettings => tr("Save identity"),
                        DialogKind::Push => tr("Push"),
                        DialogKind::Pull => tr("Pull"),
                        DialogKind::Fetch => tr("Fetch"),
                        DialogKind::NewBranch => tr("Create branch"),
                        DialogKind::TrackBranch { .. } => tr("Create and switch"),
                        DialogKind::Rename(_) => tr("Rename branch"),
                        DialogKind::DeleteBranches => tr("Delete branches"),
                        DialogKind::RunCommand(index) => if self.commands.get(index).is_some_and(|c|c.action==rocket_git::CommandAction::Url) {tr("Open URL")} else {tr("Run")},
                        DialogKind::Stash => tr("Save stash"),
                        DialogKind::Merge => tr("Merge branch"),
                        DialogKind::CloseTab(_) => tr("Close repository"),
                        DialogKind::Confirm(Action::DeleteBranch(_), _) => tr("Delete branch"),
                        DialogKind::Confirm(Action::DropStash(_), _) => tr("Drop stash"),
                        DialogKind::Confirm(Action::ApplyStash { .. }, _) => tr("Apply stash"),
                        DialogKind::Confirm(Action::SaveConflict { stage, .. }, _) => {
                            if stage {
                                tr("Save and stage")
                            } else {
                                tr("Save file")
                            }
                        }
                        DialogKind::ConflictReload(_) => tr("Reload"),
                        DialogKind::Confirm(Action::KeepConflictSide { keep, .. }, _) => {
                            match keep {
                                rocket_git::ConflictKeep::Ours => tr("Keep ours"),
                                rocket_git::ConflictKeep::Theirs => tr("Keep theirs"),
                                rocket_git::ConflictKeep::Deletion => tr("Keep deletion"),
                            }
                        }
                        DialogKind::Confirm(Action::AbortMerge, _) => tr("Abort merge"),
                        DialogKind::Confirm(Action::FinishMerge, _) => tr("Clear merge state"),
                        DialogKind::Confirm(Action::RemoveRemote(_), _) => tr("Remove remote"),
                        DialogKind::AddRemote => tr("Add remote"),
                        DialogKind::EditRemote(..) => tr("Save URL"),
                        DialogKind::Quit => tr("Exit"),
                        _ => tr("Confirm"),
                    };
                    let label = match dialog.kind {
                        DialogKind::DeleteBranches => crate::small_dialogs::delete_footer(&dialog).0,
                        DialogKind::Quit if self.write.is_some() => tr("Stop and exit").into(),
                        _ => label.to_owned(),
                    };
                    let button = if danger {
                        pages::danger(ui, ready, &label)
                    } else {
                        pages::primary(ui, ready, &label)
                    };
                    pages::control(widgets, "dialog-submit", &button, ui.clip_rect());
                    submit |= button.clicked();
                    if ready && ui.input_mut(|i| i.consume_key(crate::shortcuts::primary_modifiers(), Key::Enter)) {
                        submit = true;
                    }
                    let status = crate::small_dialogs::status(&dialog, tab);
                    if !status.is_empty() {
                        pages::footer_status(ui, false, &status);
                    }
                    // Run custom command says where the run stands, left of its buttons.
                    if let DialogKind::RunCommand(index) = dialog.kind {
                        let status = if self.write.is_some() {
                            tr("Wait for the current operation to finish").to_owned()
                        } else {
                            let context = self
                                .tabs
                                .iter()
                                .find(|t| Some(t.id) == dialog.owner)
                                .and_then(|t| crate::custom_commands::selected_context(t, dialog.command_context.as_ref()));
                            crate::custom_commands::run_status(
                                self.commands.get(index),
                                context.as_ref(),
                                &dialog.command_values,
                            )
                        };
                        ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                            ui.add(Label::new(RichText::new(status).size(12.5).color(HINT())).truncate());
                        });
                    }
                }
            };
        // About, the shortcut list and release notes have nothing to decide: the header's
        // × closes them, so they have no footer.
        let response = if informational {
            pages::info(ctx, "rocket-dialog", head, &mut self.frame_widgets, "dialog-scroll", &mut content)
        } else {
            pages::review(ctx, "rocket-dialog", head, &mut self.frame_widgets, "dialog-scroll", true, &mut content)
        };
        if informational && let Some(close) = self.frame_widgets.get("dialog-close").copied() {
            // Automation closed these with their Done button.
            self.frame_widgets.insert("dialog-submit".into(), close);
        }
        if response.should_close() {
            dismiss = true;
        }
        // Add remote from an empty Fetch or Push, or Pull from Push, opens in its place.
        if let Some(kind) = dialog.small.open.take() {
            self.show_dialog(kind);
            return;
        }
        if submit {
            let owner = dialog.owner;
            let mut action = None;
            match dialog.kind.clone() {
                DialogKind::RepositorySettings
                    if dialog.issue_links.as_ref().is_some_and(|e| e.shown) =>
                {
                    if let Some(editor) = &dialog.issue_links
                        && let Some(expected) = &editor.settings
                    {
                        match editor.problem() {
                            Some(problem) => dialog.error = Some(problem),
                            None => {
                                action = Some(Action::SetIssueLinks {
                                    expected: expected.clone(),
                                    rules: editor.rules.clone(),
                                })
                            }
                        }
                    }
                }
                DialogKind::RepositorySettings => {
                    let identity = rocket_git::LocalIdentity {
                        name: (!dialog.first.is_empty()).then(|| dialog.first.clone()),
                        email: (!dialog.second.is_empty()).then(|| dialog.second.clone()),
                    };
                    match identity.validate() {
                        Err(error) => dialog.error = Some(crate::backend::user_error(&error)),
                        Ok(()) => {
                            if let Some(expected) = &dialog.identity {
                                action = Some(Action::SetIdentity {
                                    expected: expected.clone(),
                                    identity,
                                });
                            }
                        }
                    }
                }
                DialogKind::Open => {
                    self.open(PathBuf::from(dialog.first.trim()));
                    dismiss = true;
                }
                DialogKind::NewBranch => action = Some(Action::CreateBranch(dialog.first.clone())),
                DialogKind::TrackBranch { upstream, .. } => {
                    action = Some(Action::CheckOutRemote {
                        name: dialog.first.trim().as_bytes().to_vec(),
                        upstream,
                    })
                }
                DialogKind::Rename(old) => {
                    action = Some(Action::RenameBranch(old, dialog.first.clone()))
                }
                DialogKind::AddRemote => {
                    action = Some(Action::AddRemote {
                        name: dialog.first.as_bytes().to_vec(),
                        url: dialog.second.as_bytes().to_vec(),
                    })
                }
                DialogKind::EditRemote(expected, kind) => {
                    action = Some(Action::EditRemote {
                        expected,
                        kind,
                        url: (!(kind == rocket_git::RemoteUrlKind::Push && dialog.flag))
                            .then(|| dialog.first.as_bytes().to_vec()),
                    })
                }
                DialogKind::Fetch => {
                    if let Some(remote) = dialog.remotes.get(dialog.remote) {
                        action = Some(Action::FetchWithTags(remote.clone(),dialog.fetch_tags));
                    }
                }
                DialogKind::Pull => {} // Dedicated reviewed flow; no one-click transport here.
                DialogKind::Push => {
                    if let Some(remote) = dialog.remotes.get(dialog.remote) {
                        action = Some(Action::Push {
                            remote: remote.clone(),
                            local: dialog.branch.clone(),
                            destination: dialog.second.as_bytes().to_vec(),
                            set_upstream: dialog.flag,
                        });
                    }
                }
                DialogKind::DeleteBranches => {
                    action = Some(Action::DeleteBranches(dialog.selected_branches.clone()))
                }
                DialogKind::RunCommand(index) => {
                    if let Some(url) = crate::custom_commands::command_url(
                        &self.commands,
                        &self.tabs,
                        dialog.owner,
                        index,
                        &dialog.command_values,
                        dialog.command_context.as_ref(),
                    ) {
                        self.open_reviewed_url(&url, "custom-url-open");
                        dismiss = true;
                    }
                    if let Some((name, program, arguments)) = crate::custom_commands::command_line(
                        &self.commands,
                        &self.tabs,
                        dialog.owner,
                        index,
                        &dialog.command_values,
                        dialog.command_context.as_ref(),
                    ) {
                        action = Some(Action::RunCommand {
                            name,
                            program,
                            arguments,
                            storage: self.state_directory.join("commands"),
                        });
                    }
                }
                DialogKind::Stash => {
                    action = Some(Action::Stash {
                        message: dialog.first.clone(),
                        include_untracked: dialog.flag,
                        paths: dialog.stash_paths.clone().unwrap_or_default(),
                    })
                }
                DialogKind::Merge => unreachable!("dedicated merge review"),
                DialogKind::Confirm(mut selected, _) => {
                    if let Action::ApplyStash { restore_index, .. } = &mut selected {
                        *restore_index = dialog.flag;
                    }
                    action = Some(selected);
                }
                DialogKind::CloseTab(id) => {
                    self.remove_tab(id);
                    dismiss = true;
                }
                DialogKind::ConflictReload(path) => {
                    if let Some(owner) = owner {
                        self.load_conflict(owner, path);
                    }
                    dismiss = true;
                }
                DialogKind::Quit => {
                    self.begin_quit();
                    dismiss = true;
                }
                _ => dismiss = true,
            }
            if let Some(action) = action {
                if let Some(owner) = owner {
                    let conflict = matches!(
                        action,
                        Action::SaveConflict { .. } | Action::KeepConflictSide { .. }
                    );
                    self.execute(owner, action);
                    if !conflict {
                        self.show_console = true;
                    }
                    dismiss = true;
                } else {
                    dialog.error = Some(tr("Open a repository first.").into());
                }
            }
        }
        if dismiss
            && !submit
            && matches!(
                dialog.kind,
                DialogKind::Confirm(Action::SaveConflict { .. }, _)
            )
        {
            self.conflict_save_cancelled(dialog.owner);
        }
        if dismiss
            && !submit
            && matches!(
                dialog.kind,
                DialogKind::Confirm(Action::KeepConflictSide { .. }, _)
            )
        {
            self.conflict_keep_cancelled(dialog.owner);
        }
        if !dismiss {
            self.dialog = Some(dialog);
            if let Some((index, current)) = command_browse {
                self.browse(crate::folder_browser::PickTarget::CommandInput(index), &current);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        app::{DialogKind, Rocket},
        backend::Action,
        test_common as common,
    };
    use eframe::egui::{Context, Event, Modifiers, MouseWheelUnit, PointerButton, Pos2, RawInput, Rect, TouchPhase, Vec2, vec2};

    #[cfg(windows)]
    #[test]
    fn accounts_and_ssh_keys_open_in_preferences_without_a_repository() {
        common::isolated(
            "ROCKET_MENUS_UI_CASE",
            "menus::tests::accounts_and_ssh_keys_open_in_preferences_without_a_repository",
            || {
                use crate::batch::flow_tests::{click, enabled, wait};
                use crate::{forge::Section, preferences::Page};
                let fixture = common::Fixture::new();
                let ctx = Context::default();
                let mut app = Rocket::with_log_directory(&ctx, vec![], fixture.root.join("logs"));
                app.ssh.root = std::fs::canonicalize(&fixture.root).unwrap().join("ssh-home/rocket-keys");
                // A disposable SSH folder: nothing of the user's own is read.
                app.ssh.scan_directory = app.ssh.root.parent().unwrap().to_path_buf();
                wait(&ctx, &mut app);
                assert!(app.tabs.is_empty());
                // The File menu no longer lists them.
                click(&ctx, &mut app, "menu-file");
                assert!(!app.frame_widgets.contains_key("file-accounts"));
                assert!(!app.frame_widgets.contains_key("file-ssh-keys"));
                click(&ctx, &mut app, "menu-edit");
                click(&ctx, &mut app, "preferences");
                click(&ctx, &mut app, "pref-page-accounts");
                wait(&ctx, &mut app);
                assert_eq!(app.preferences_page(), Some(Page::Accounts));
                assert!(enabled(&ctx, &mut app, "hosting-provider-gitlab"));
                assert!(app.forge.accounts_loaded && app.forge.accounts.is_empty());
                let remember = app.frame_widgets["hosting-remember"].rect;
                let connect = app.frame_widgets["hosting-connect"].rect;
                assert!(connect.top() <= remember.bottom() + 20.0, "Account actions must stay next to the form: {connect:?} below {remember:?}");
                // Opened from Preferences, it stays in Preferences; Hosting is not involved.
                assert_eq!(app.forge.section, Section::Pulls);
                click(&ctx, &mut app, "pref-page-ssh");
                wait(&ctx, &mut app);
                assert_eq!(app.preferences_page(), Some(Page::SshKeys));
                assert!(enabled(&ctx, &mut app, "ssh-new"));
                assert!(!app.frame_widgets.contains_key("ssh-restore"), "nothing to restore without a repository");
                assert!(app.ssh.error.is_none(), "{:?}", app.ssh.error);
                assert!(app.ssh.loaded && app.ssh.profiles.is_empty());
                click(&ctx, &mut app, "dialog-cancel");
                assert!(app.dialog.is_none());
                assert!(app.tabs.is_empty());
                assert!(!fixture.root.join("ssh-home/rocket-keys").exists());
            },
        );
    }

    /// The 13 Repository rows all show in a tall window: egui does not hold the menu at
    /// its first popup size. Items wait in their submenu, which opens beside the menu.
    /// A short window scrolls the menu, and its last row stays reachable.
    #[test]
    fn repository_menu_groups_items_into_submenus() {
        common::isolated(
            "ROCKET_MENUS_UI_CASE",
            "menus::tests::repository_menu_groups_items_into_submenus",
            || {
                let fixture = common::Fixture::new();
                fixture.write("file", "first");
                fixture.commit("first");
                let ctx = Context::default();
                let mut app = crate::batch::flow_tests::open(&ctx, &fixture);
                let frame = |app: &mut Rocket, size: Vec2, events: Vec<Event>| {
                    app.frame_widgets.clear();
                    let mut output = ctx.run_ui(
                        RawInput {
                            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
                            events,
                            ..Default::default()
                        },
                        |ui| app.shell(ui, &mut Vec::new()),
                    );
                    output.textures_delta.clear();
                };
                // Clicking the title opens the menu, or closes it when it shows.
                let title = |app: &mut Rocket, size: Vec2| {
                    frame(app, size, vec![]);
                    let pos = app.frame_widgets["menu-repository"].rect.center();
                    let press = |pressed| Event::PointerButton {
                        pos,
                        button: PointerButton::Primary,
                        pressed,
                        modifiers: Modifiers::NONE,
                    };
                    frame(app, size, vec![Event::PointerMoved(pos), press(true), press(false)]);
                    for _ in 0..4 {
                        frame(app, size, vec![]);
                    }
                };
                let tall = vec2(1480.0, 1300.0);
                title(&mut app, tall);
                let menu = app.frame_widgets["repository-menu-scroll"].rect;
                assert!(menu.bottom() <= tall.y, "menu extends outside window: {menu:?}");
                assert!(menu.height() < 600.0, "menu is taller than its rows: {menu:?}");
                // Fetch, Pull and Push have no ids; every other row does.
                for key in [
                    "repository-atomic-push",
                    "repository-commit-menu",
                    "repository-branch-menu",
                    "repository-gitflow",
                    "repository-stash-menu",
                    "repository-patches-menu",
                    "repository-history-menu",
                    "repository-custom-commands",
                    "repository-remotes-menu",
                    "repository-settings-menu",
                ] {
                    let row = app.frame_widgets.get(key).unwrap_or_else(|| panic!("{key} is not shown")).rect;
                    assert!(row.height() >= 28.0 && menu.contains_rect(row), "{key}: {row:?} in {menu:?}");
                }
                assert!(!app.frame_widgets.contains_key("repository-settings"), "items wait in their submenu");
                let row = app.frame_widgets["repository-settings-menu"].rect;
                frame(&mut app, tall, vec![Event::PointerMoved(row.center())]);
                for _ in 0..3 {
                    frame(&mut app, tall, vec![]);
                }
                let item = app.frame_widgets.get("repository-settings").expect("the submenu shows its rows").rect;
                assert!(item.left() >= menu.right() - 1.0, "{item:?} beside {menu:?}");
                assert!(app.frame_widgets["repository-ignore"].enabled);
                title(&mut app, tall);
                assert!(!app.frame_widgets.contains_key("repository-menu-scroll"), "menu closed");
                let short = vec2(1480.0, 420.0);
                title(&mut app, short);
                let menu = app.frame_widgets["repository-menu-scroll"].rect;
                assert!(menu.bottom() <= short.y, "menu extends outside window: {menu:?}");
                frame(&mut app, short, vec![
                    Event::PointerMoved(menu.center()),
                    Event::MouseWheel { unit: MouseWheelUnit::Point, delta: vec2(0.0, -2000.0), phase: TouchPhase::Move, source: eframe::egui::MouseWheelSource::Unknown, modifiers: Modifiers::NONE },
                ]);
                for _ in 0..12 {
                    frame(&mut app, short, vec![]);
                }
                let last = app
                    .frame_widgets
                    .get("repository-settings-menu")
                    .expect("the last row is reachable after scrolling")
                    .rect;
                assert!(last.height() >= 28.0 && menu.contains_rect(last), "{last:?} in {menu:?}");
            },
        );
    }

    /// Edit's text commands act on the field the menu was opened from. Opening the menu
    /// takes that field's focus; the command still reaches it a frame later.
    #[test]
    fn edit_menu_acts_on_the_field_it_was_opened_from() {
        use eframe::egui::{Id, Key, OutputCommand, TextEdit};
        common::isolated(
            "ROCKET_MENUS_UI_CASE",
            "menus::tests::edit_menu_acts_on_the_field_it_was_opened_from",
            || {
                let fixture = common::Fixture::new();
                fixture.write("file", "first");
                fixture.commit("first");
                fixture.write("file", "second");
                let ctx = Context::default();
                let mut app = crate::batch::flow_tests::open(&ctx, &fixture);
                app.dispatch(crate::app::Intent::View(crate::app::View::Changes));
                // As in the app frame: the Edit command chosen last frame, then the window.
                let frame = |app: &mut Rocket, events: Vec<Event>| {
                    app.frame_widgets.clear();
                    let mut output = ctx.run_ui(
                        RawInput {
                            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1480.0, 1000.0))),
                            events,
                            ..Default::default()
                        },
                        |ui| {
                            app.apply_text_command(ui.ctx());
                            let mut intents = Vec::new();
                            app.shell(ui, &mut intents);
                            for intent in intents {
                                app.dispatch(intent);
                            }
                        },
                    );
                    output.textures_delta.clear();
                    std::mem::take(&mut output.platform_output.commands)
                };
                // Returns what the frame after the click put on the clipboard.
                let click = |app: &mut Rocket, key: &str| {
                    frame(app, vec![]);
                    let control = app.frame_widgets.get(key).copied().unwrap_or_else(|| panic!("{key} is not shown"));
                    assert!(control.enabled, "{key} is disabled");
                    let pos = control.rect.center();
                    let press = |pressed| Event::PointerButton {
                        pos,
                        button: PointerButton::Primary,
                        pressed,
                        modifiers: Modifiers::NONE,
                    };
                    frame(app, vec![Event::PointerMoved(pos), press(true), press(false)]);
                    frame(app, vec![])
                        .into_iter()
                        .filter_map(|command| match command {
                            OutputCommand::CopyText(text) => Some(text),
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                };
                let draft = |app: &Rocket| app.tab().unwrap().draft.clone();
                let field = Id::unique(("commit-message", app.tab().unwrap().id));

                click(&mut app, "commit-message");
                frame(&mut app, vec![Event::Text("Fix sign-in".into())]);
                assert_eq!(draft(&app), "Fix sign-in");
                click(&mut app, "menu-edit");
                assert!(!ctx.memory(|m| m.has_focus(field)), "the menu took the field's focus");
                for key in ["edit-undo", "edit-redo", "edit-paste", "edit-select-all"] {
                    assert!(app.frame_widgets[key].enabled, "{key} while typing");
                }
                for key in ["edit-cut", "edit-copy"] {
                    assert!(!app.frame_widgets[key].enabled, "{key} needs selected text");
                }
                click(&mut app, "edit-undo");
                assert_eq!(draft(&app), "");
                assert!(ctx.memory(|m| m.has_focus(field)), "the field has focus again");
                click(&mut app, "menu-edit");
                click(&mut app, "edit-redo");
                assert_eq!(draft(&app), "Fix sign-in");
                click(&mut app, "menu-edit");
                click(&mut app, "edit-select-all");
                let range = TextEdit::load_state(&ctx, field).and_then(|s| s.cursor.char_range()).expect("a selection");
                let (a, b) = (range.primary.index.0, range.secondary.index.0);
                assert_eq!((a.min(b), a.max(b)), (0, 11));
                click(&mut app, "menu-edit");
                assert_eq!(click(&mut app, "edit-copy"), ["Fix sign-in"]);
                assert_eq!(draft(&app), "Fix sign-in");
                click(&mut app, "menu-edit");
                assert_eq!(click(&mut app, "edit-cut"), ["Fix sign-in"]);
                assert_eq!(draft(&app), "");
                // With no field focused the text commands are grey.
                let escape = Event::Key { key: Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE };
                frame(&mut app, vec![escape]);
                click(&mut app, "menu-edit");
                for key in ["edit-undo", "edit-redo", "edit-cut", "edit-copy", "edit-paste", "edit-select-all"] {
                    assert!(!app.frame_widgets[key].enabled, "{key} without a focused field");
                }
                assert!(!app.frame_widgets["edit-undo-commit"].enabled, "no commit made in this session");
            },
        );
    }

    /// A hovered menu row gets the bright band and white text; resting rows have neither.
    #[test]
    fn hovered_menu_row_is_lit_with_white_text() {
        use eframe::egui::{Shape, epaint::ClippedShape};
        common::isolated(
            "ROCKET_MENUS_UI_CASE",
            "menus::tests::hovered_menu_row_is_lit_with_white_text",
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
                // Open Repository is enabled once startup has found Git.
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(45);
                while app.busy() {
                    app.poll(&ctx);
                    assert!(std::time::Instant::now() < deadline);
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                let frame = |app: &mut Rocket, events: Vec<Event>| {
                    app.frame_widgets.clear();
                    let mut output = ctx.run_ui(
                        RawInput {
                            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 800.0))),
                            events,
                            ..Default::default()
                        },
                        |ui| app.shell(ui, &mut Vec::new()),
                    );
                    output.textures_delta.clear();
                    output.shapes
                };
                frame(&mut app, vec![]);
                let pos = app.frame_widgets["menu-file"].rect.center();
                let press = |pressed| Event::PointerButton {
                    pos,
                    button: PointerButton::Primary,
                    pressed,
                    modifiers: Modifiers::NONE,
                };
                frame(&mut app, vec![Event::PointerMoved(pos), press(true), press(false)]);
                frame(&mut app, vec![]);
                let row = app.frame_widgets["file-open"];
                assert!(row.enabled);
                // Whether the row's band is painted, and its label's colour.
                let lit = |shapes: &[ClippedShape]| {
                    let band = shapes.iter().any(|s| {
                        matches!(&s.shape, Shape::Rect(r) if r.fill == super::MENU_HOT() && r.rect == row.rect)
                    });
                    let ink = shapes.iter().find_map(|s| match &s.shape {
                        Shape::Text(t) if t.galley.text() == "Open Repository" => t.override_text_color,
                        _ => None,
                    });
                    (band, ink)
                };
                assert_eq!(lit(&frame(&mut app, vec![])), (false, Some(crate::style::TEXT())));
                frame(&mut app, vec![Event::PointerMoved(row.rect.center())]);
                assert_eq!(lit(&frame(&mut app, vec![])), (true, Some(super::MENU_HOT_TEXT())));
                // A submenu's row stays lit while its submenu shows.
                let pos = app.frame_widgets["menu-repository"].rect.center();
                let click = |pressed| Event::PointerButton {
                    pos,
                    button: PointerButton::Primary,
                    pressed,
                    modifiers: Modifiers::NONE,
                };
                frame(&mut app, vec![Event::PointerMoved(pos), click(true), click(false)]);
                frame(&mut app, vec![]);
                let row = app.frame_widgets["repository-custom-commands"].rect;
                frame(&mut app, vec![Event::PointerMoved(row.center())]);
                let mut shapes = Vec::new();
                for _ in 0..3 {
                    shapes = frame(&mut app, vec![]);
                }
                let lit = |shapes: &[ClippedShape], rect: Rect, label: &str| {
                    let band = shapes.iter().any(|s| {
                        matches!(&s.shape, Shape::Rect(r) if r.fill == super::MENU_HOT() && r.rect == rect)
                    });
                    let ink = shapes.iter().find_map(|s| match &s.shape {
                        Shape::Text(t) if t.galley.text() == label => t.override_text_color,
                        _ => None,
                    });
                    (band, ink)
                };
                assert!(app.frame_widgets.contains_key("command-edit"), "submenu shows");
                assert_eq!(lit(&shapes, row, "Custom Commands"), (true, Some(super::MENU_HOT_TEXT())));
                // Still lit with the pointer in its submenu.
                let edit = app.frame_widgets["command-edit"].rect;
                frame(&mut app, vec![Event::PointerMoved(edit.center())]);
                let shapes = frame(&mut app, vec![]);
                assert_eq!(lit(&shapes, row, "Custom Commands"), (true, Some(super::MENU_HOT_TEXT())));
                assert_eq!(lit(&shapes, edit, "Edit Custom Commands"), (true, Some(super::MENU_HOT_TEXT())));
            },
        );
    }

    /// About, the shortcut list and release notes have nothing to decide, so only the
    /// header's × closes them; their old Done id is that ×. The Window menu no longer
    /// offers MCP Server Settings (Preferences has them).
    #[test]
    fn info_dialogs_close_with_the_header_button_and_window_drops_mcp() {
        common::isolated(
            "ROCKET_MENUS_UI_CASE",
            "menus::tests::info_dialogs_close_with_the_header_button_and_window_drops_mcp",
            || {
                let fixture = common::Fixture::new();
                let ctx = Context::default();
                let mut app = Rocket::with_log_directory(&ctx, vec![], fixture.root.join("logs"));
                let frame = |app: &mut Rocket, events: Vec<Event>| {
                    app.frame_widgets.clear();
                    let mut output = ctx.run_ui(
                        RawInput {
                            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 860.0))),
                            events,
                            ..Default::default()
                        },
                        |ui| {
                            app.shell(ui, &mut Vec::new());
                            let ctx = ui.ctx().clone();
                            app.dialog_ui(&ctx);
                        },
                    );
                    output.textures_delta.clear();
                };
                let click = |app: &mut Rocket, id: &str| {
                    let pos = app.frame_widgets[id].rect.center();
                    let press = |pressed| Event::PointerButton {
                        pos,
                        button: PointerButton::Primary,
                        pressed,
                        modifiers: Modifiers::NONE,
                    };
                    frame(app, vec![Event::PointerMoved(pos), press(true)]);
                    frame(app, vec![press(false)]);
                };
                for kind in [DialogKind::About, DialogKind::Legal, DialogKind::Changelog] {
                    app.show_dialog(kind);
                    // A new modal is laid out a frame before it is drawn.
                    frame(&mut app, vec![]);
                    frame(&mut app, vec![]);
                    let close = app.frame_widgets["dialog-close"].rect;
                    assert_eq!(app.frame_widgets["dialog-submit"].rect, close, "no Done beside ×");
                    assert_eq!(app.frame_widgets["dialog-cancel"].rect, close, "no Cancel beside ×");
                    click(&mut app, "dialog-close");
                    assert!(app.dialog.is_none(), "× closes it");
                }
                frame(&mut app, vec![]);
                click(&mut app, "menu-window");
                frame(&mut app, vec![]);
                for id in ["window-next", "window-previous"] {
                    assert!(app.frame_widgets.contains_key(id), "the open Window menu has {id}");
                }
                assert!(!app.frame_widgets.contains_key("menu-mcp"), "Window has no MCP Server Settings");
            },
        );
    }

    #[test]
    fn only_destructive_decisions_use_the_danger_style() {
        common::isolated(
            "ROCKET_MENUS_UI_CASE",
            "menus::tests::only_destructive_decisions_use_the_danger_style",
            || {
                let fixture = common::Fixture::new();
                let ctx = Context::default();
                let mut app = Rocket::with_log_directory(&ctx, vec![], fixture.root.join("logs"));
                let mut danger = |kind: DialogKind| {
                    app.dialog = None;
                    app.show_dialog(kind);
                    app.dialog_danger(app.dialog.as_ref().expect("dialog opened"))
                };
                let remote = rocket_git::Remote {
                    name: b"origin".to_vec(),
                    urls: vec![b"https://example.invalid/repo.git".to_vec()],
                    push_urls: vec![],
                    fetch_refspecs: vec![],
                };
                for kind in [
                    DialogKind::Confirm(Action::DeleteBranch(b"topic".to_vec()), String::new()),
                    DialogKind::Confirm(Action::RemoveRemote(remote), String::new()),
                    DialogKind::Confirm(Action::AbortMerge, String::new()),
                    DialogKind::DeleteBranches,
                ] {
                    assert!(danger(kind), "a destructive confirmation is not red");
                }
                for kind in [
                    DialogKind::NewBranch,
                    DialogKind::Clone,
                    DialogKind::Push,
                    DialogKind::Merge,
                    DialogKind::Stash,
                    DialogKind::Quit,
                    DialogKind::Preferences,
                ] {
                    assert!(!danger(kind), "a non-destructive dialog is red");
                }
            },
        );
    }
}
