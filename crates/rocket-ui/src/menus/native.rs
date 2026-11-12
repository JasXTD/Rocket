//! Native rows share the reviewed dialogs and dispatcher with the desktop menus.
//! IDs are fixed: a queued Cocoa action cannot become a different command after refresh.
use super::*;
use crate::shortcuts::model::{Action as Shortcut, COMMANDS, Trigger};
use rocket_git::macos_menu::{self, Item};

enum Action {
    Shortcut(Shortcut),
    Intent(Intent),
    Text(TextCommand),
    Updates,
    Guide,
    Palette,
    Terminal(TerminalAction),
    Group,
}
struct Row {
    item: Item,
    target: String,
    action: Action,
}
impl Row {
    fn new(
        id: u32,
        group: u32,
        parent: u32,
        target: &str,
        title: &'static str,
        enabled: bool,
        action: Action,
    ) -> Self {
        Self {
            item: Item {
                id,
                group,
                parent,
                title: tr(title).into(),
                key: String::new(),
                modifiers: 0,
                enabled,
                checked: false,
            },
            target: target.into(),
            action,
        }
    }
}

/// Cocoa's documented function-key Unicode range, independent of keyboard layout.
fn equivalent(key: Key) -> Option<String> {
    let code = match key {
        Key::ArrowUp => 0xf700,
        Key::ArrowDown => 0xf701,
        Key::ArrowLeft => 0xf702,
        Key::ArrowRight => 0xf703,
        Key::Insert => 0xf727,
        Key::Delete => 0xf728,
        Key::Home => 0xf729,
        Key::End => 0xf72b,
        Key::PageUp => 0xf72c,
        Key::PageDown => 0xf72d,
        Key::Backspace => 8,
        Key::Tab => 9,
        Key::Enter => 13,
        Key::Escape => 27,
        Key::Space => 32,
        _ => {
            let name = key.name();
            if let Some(n) = name
                .strip_prefix('F')
                .and_then(|s| s.parse::<u32>().ok())
                .filter(|n| (1..=35).contains(n))
            {
                0xf703 + n
            } else if name.len() == 1 {
                return Some(name.to_lowercase());
            } else {
                return None;
            }
        }
    };
    char::from_u32(code).map(|c| c.to_string())
}

impl Rocket {
    fn native_rows(&self, ctx: &Context) -> Vec<Row> {
        let mut rows = Vec::new();
        for (index, command) in COMMANDS.iter().enumerate() {
            let group = match command.action {
                Shortcut::Preferences => 0,
                Shortcut::New
                | Shortcut::Open
                | Shortcut::Clone
                | Shortcut::Explore
                | Shortcut::Manager
                | Shortcut::Close
                | Shortcut::Reopen => 1,
                Shortcut::Next | Shortcut::Previous => 6,
                Shortcut::Terminal => 5,
                Shortcut::Commit
                | Shortcut::Branch
                | Shortcut::Refresh
                | Shortcut::Fetch
                | Shortcut::Pull
                | Shortcut::Push => 4,
                _ => 3,
            };
            let target = match command.action {
                Shortcut::Preferences => "preferences".into(),
                Shortcut::Open => "file-open".into(),
                _ => format!("shortcut-{}", command.id),
            };
            let mut row = Row::new(
                index as u32 + 1,
                group,
                0,
                &target,
                command.label,
                self.shortcut_available(command.action),
                Action::Shortcut(command.action),
            );
            if !self.keybindings.capturing()
                && (!self.terminal_focused(ctx) || command.action == Shortcut::Terminal)
            {
                if let Some((key, modifiers)) = self
                    .keybindings
                    .bindings
                    .get(command.action)
                    .into_iter()
                    .find_map(|binding| {
                        let Trigger::Key(key) = binding.trigger else {
                            return None;
                        };
                        equivalent(key).map(|k| (k, u32::from(binding.modifiers)))
                    })
                {
                    row.item.key = key;
                    row.item.modifiers = modifiers;
                }
            }
            row.item.checked = command.action == Shortcut::Sidebar && self.show_sidebar;
            rows.push(row);
        }
        let available = !self.quitting && !self.blocking_dialog();
        for (id, group, target, title, action) in [
            (
                9001,
                0,
                "help-about",
                "About Rocket",
                Action::Intent(Intent::Dialog(DialogKind::About)),
            ),
            (
                9002,
                7,
                "help-updates",
                "Check for Updates",
                Action::Updates,
            ),
            (
                9003,
                0,
                "file-exit",
                "Quit Rocket",
                Action::Intent(Intent::Exit),
            ),
            (9004, 7, "help-guide", "Guide", Action::Guide),
            (
                9005,
                7,
                "help-changelog",
                "Release Notes",
                Action::Intent(Intent::Dialog(DialogKind::Changelog)),
            ),
            (9006, 4, "repository-more", "More Commands", Action::Palette),
        ] {
            let mut row = Row::new(
                id,
                group,
                0,
                target,
                title,
                available || (id == 9003 && !self.quitting),
                action,
            );
            if id == 9003 && !self.keybindings.capturing() {
                row.item.key = "q".into();
                row.item.modifiers = 8;
            }
            rows.push(row);
        }
        let text_target = focused_text_field(ctx);
        for (index, (command, title, key)) in [
            (TextCommand::Undo, "Undo", "z"),
            (TextCommand::Redo, "Redo", "z"),
            (TextCommand::Cut, "Cut", "x"),
            (TextCommand::Copy, "Copy", "c"),
            (TextCommand::Paste, "Paste", "v"),
            (TextCommand::SelectAll, "Select All", "a"),
        ]
        .into_iter()
        .enumerate()
        {
            let enabled = text_target.is_some_and(|t| {
                !matches!(command, TextCommand::Cut | TextCommand::Copy)
                    || t.selection.is_some_and(|r| !r.is_empty())
            });
            let mut row = Row::new(
                9100 + index as u32,
                2,
                0,
                &format!("edit-{}", title.to_lowercase().replace(' ', "-")),
                title,
                enabled,
                Action::Text(command),
            );
            if !self.keybindings.capturing() {
                row.item.key = key.into();
            }
            row.item.modifiers = if command == TextCommand::Redo { 12 } else { 8 };
            rows.push(row);
        }
        let opened = available && self.tab().is_some();
        let idle = self.write.is_none();
        let writable = available && self.can_write();
        let state = self
            .tab()
            .and_then(|t| t.snapshot.as_ref())
            .and_then(|s| s.state.as_ref());
        let settled = state.is_some_and(|s| s.is_idle());
        for (id, target, title) in [
            (10000, "repository-commit-menu", "Commit"),
            (10001, "repository-branch-menu", "Branch"),
            (10002, "repository-stash-menu", "Stash"),
            (10003, "repository-patches-menu", "Patches"),
            (10004, "repository-history-menu", "History & Recovery"),
            (10005, "repository-settings-menu", "Settings"),
        ] {
            rows.push(Row::new(id, 4, 0, target, title, opened, Action::Group));
        }
        let mut add = |id, parent, target, title, enabled, intent| {
            rows.push(Row::new(
                id,
                4,
                parent,
                target,
                title,
                enabled,
                Action::Intent(intent),
            ))
        };
        add(
            10100,
            0,
            "repository-atomic-push",
            "Push Branches Together",
            writable,
            Intent::Dialog(DialogKind::AtomicPush),
        );
        for (n, operation, target, title) in [
            (
                0,
                rocket_git::CommitOperation::Amend,
                "commit-amend",
                "Amend Latest Commit",
            ),
            (
                1,
                rocket_git::CommitOperation::CherryPick,
                "commit-cherry-pick",
                "Cherry-pick Selected Commits",
            ),
            (
                2,
                rocket_git::CommitOperation::Revert,
                "commit-revert",
                "Revert Selected Commits",
            ),
        ] {
            let targets = if operation == rocket_git::CommitOperation::Amend {
                Vec::new()
            } else {
                self.tab()
                    .map_or_else(Vec::new, |t| t.commit_targets(operation))
            };
            let ready = operation == rocket_git::CommitOperation::Amend || !targets.is_empty();
            add(
                10110 + n,
                10000,
                target,
                title,
                writable && settled && ready,
                Intent::Dialog(DialogKind::CommitAction(rocket_git::CommitActionRequest {
                    operation,
                    targets,
                    mainline: None,
                    recovery: None,
                })),
            );
        }
        for (id, parent, target, title, enabled, dialog) in [
            (
                10120,
                10001,
                "repository-merge",
                "Merge Branch",
                writable,
                DialogKind::Merge,
            ),
            (
                10121,
                10001,
                "repository-rebase",
                "Rebase",
                writable && settled,
                DialogKind::Rebase(rocket_git::RebaseAction::Start),
            ),
            (
                10122,
                10001,
                "repository-interactive-rebase",
                "Interactive Rebase",
                writable && settled,
                DialogKind::Rebase(rocket_git::RebaseAction::Interactive),
            ),
            (
                10123,
                10001,
                "repository-delete-branches",
                "Delete Branches",
                writable,
                DialogKind::DeleteBranches,
            ),
            (
                10124,
                10001,
                "repository-workflows",
                "Workflows & Cleanup",
                writable,
                DialogKind::Workflows,
            ),
            (
                10130,
                10002,
                "repository-stash",
                "Save Changes",
                writable,
                DialogKind::Stash,
            ),
            (
                10140,
                10003,
                "repository-patch-import",
                "Import Patch",
                opened && !self.busy(),
                DialogKind::Patch(crate::patches::Mode::Import),
            ),
            (
                10141,
                10003,
                "repository-patch-export",
                "Export Patch",
                opened && !self.busy(),
                DialogKind::Patch(crate::patches::Mode::Export),
            ),
            (
                10142,
                10003,
                "repository-series-import",
                "Import Commit Series",
                writable && !self.busy(),
                DialogKind::MailImport(rocket_git::MailImportAction::Start),
            ),
            (
                10150,
                10004,
                "repository-recovery",
                "Reflog & Recovery",
                opened && !self.busy(),
                DialogKind::Recovery,
            ),
            (
                10151,
                10004,
                "repository-file-backups",
                "File Backups & Restore",
                opened && !self.busy(),
                DialogKind::FileEdit(None),
            ),
            (
                10152,
                10004,
                "repository-bisect",
                "Bisect",
                opened && !self.interaction_busy(),
                DialogKind::Bisect,
            ),
            (
                10154,
                10004,
                "repository-history-index",
                "Speed Up History",
                opened,
                DialogKind::HistoryIndex,
            ),
            (
                10160,
                10005,
                "repository-settings",
                "Repository Settings",
                opened && idle,
                DialogKind::RepositorySettings,
            ),
            (
                10161,
                10005,
                "repository-signing",
                "Signing & Verification",
                opened && idle,
                DialogKind::Signing(None),
            ),
            (
                10162,
                10005,
                "repository-ignore",
                "Ignored Files (.gitignore)",
                writable,
                DialogKind::Ignore(Vec::new()),
            ),
            (
                10170,
                0,
                "repository-tags",
                "Tags",
                opened,
                DialogKind::Tags,
            ),
            (
                10171,
                0,
                "repository-worktrees",
                "Worktrees",
                opened,
                DialogKind::Worktrees,
            ),
            (
                10172,
                0,
                "repository-submodules",
                "Submodules",
                opened,
                DialogKind::Submodules,
            ),
            (
                10173,
                0,
                "repository-lfs",
                "Git LFS",
                opened,
                DialogKind::Lfs,
            ),
            (
                10174,
                0,
                "repository-hosting",
                "Hosting",
                opened,
                DialogKind::Hosting,
            ),
        ] {
            add(id, parent, target, title, enabled, Intent::Dialog(dialog));
        }
        add(
            10131,
            10002,
            "repository-stashes",
            "View Stashes",
            opened,
            Intent::View(View::Stashes),
        );
        add(
            10143,
            10003,
            "repository-series-export",
            "Export Commit Series",
            opened && !self.busy(),
            Intent::ExportSeries(None),
        );
        add(
            10153,
            10004,
            "repository-tree",
            "Browse Files at Commit",
            opened && !self.interaction_busy(),
            Intent::Tree(crate::tree::TreeIntent::OpenSelected),
        );
        add(
            10163,
            10005,
            "command-edit",
            "Custom Commands",
            opened && idle,
            Intent::Preferences(crate::preferences::Page::Commands),
        );
        add(
            10164,
            10005,
            "repository-extensions",
            "Extensions",
            available,
            Intent::Preferences(crate::preferences::Page::Integrations),
        );
        add(
            10165,
            10005,
            "repository-playground",
            "Lua Playground",
            opened && !self.busy(),
            Intent::Playground,
        );
        add(
            10175,
            0,
            "repository-manage-remotes",
            "Manage Remotes",
            opened,
            Intent::View(View::Remotes),
        );
        for (n, target, title, view) in [
            (0, "goto-summary", "Repository Summary", View::Summary),
            (1, "goto-map", "Repository Map", View::Map),
            (2, "goto-compare", "Compare", View::Compare),
            (3, "goto-branches", "Branches", View::Branches),
        ] {
            let mut row = Row::new(
                10200 + n,
                3,
                0,
                target,
                title,
                opened,
                Action::Intent(Intent::View(view)),
            );
            row.item.checked = self.tab().is_some_and(|t| t.view == view);
            rows.push(row);
        }
        for (n, target, title, action) in [
            (
                0,
                "terminal-interrupt",
                "Interrupt Command",
                TerminalAction::Interrupt,
            ),
            (
                1,
                "terminal-clear",
                "Clear Scrollback",
                TerminalAction::ClearScrollback,
            ),
            (2, "terminal-end", "End Session", TerminalAction::End),
        ] {
            rows.push(Row::new(
                10300 + n,
                5,
                0,
                target,
                title,
                opened && self.tab().is_some_and(|t| t.terminal.is_some()),
                Action::Terminal(action),
            ));
        }
        rows
    }

    pub(crate) fn native_menubar(&mut self, ctx: &Context, intents: &mut Vec<Intent>) -> bool {
        let rows = self.native_rows(ctx);
        let repaint = ctx.clone();
        if !macos_menu::synchronize(
            &rows.iter().map(|r| r.item.clone()).collect::<Vec<_>>(),
            move || repaint.request_repaint(),
        ) {
            return false;
        }
        // Rebuild availability for every action, including after another queued click.
        while let Some(id) = macos_menu::next_action() {
            let Some(row) = self
                .native_rows(ctx)
                .into_iter()
                .find(|r| r.item.id == id && r.item.enabled)
            else {
                continue;
            };
            match row.action {
                Action::Shortcut(a) => self.invoke_shortcut(a, intents),
                Action::Intent(i) => intents.push(i),
                Action::Updates => self.updates.open(),
                Action::Guide => crate::guide::request(ctx),
                Action::Palette => self.palette.show(),
                Action::Terminal(a) => self.terminal_action(a),
                Action::Text(command) => {
                    if let Some(target) = focused_text_field(ctx).or(self.edit_target) {
                        if command == TextCommand::Paste {
                            ctx.send_viewport_cmd(ViewportCommand::RequestPaste);
                        }
                        self.text_command = Some((target, command));
                        ctx.request_repaint();
                    }
                }
                Action::Group => {}
            }
            // Intents are applied later in this frame. Re-evaluate the next click
            // after that transition, so two queued dialogs cannot bypass each other.
            ctx.request_repaint();
            break;
        }
        self.edit_target = focused_text_field(ctx);
        true
    }

    /// Describes the installed Cocoa controls; intentionally has no fake rectangles.
    pub fn native_menu_state(&self, ctx: &Context) -> serde_json::Value {
        let rows = self.native_rows(ctx);
        serde_json::json!(macos_menu::snapshot().iter().filter_map(|item| {
            let row = rows.iter().find(|r|r.item.id == item.id)?;
            Some(serde_json::json!({"id":row.target,"native_id":item.id,"group":item.group,"parent":item.parent,
                "title":item.title,"enabled":item.enabled && row.item.enabled,"checked":item.checked,"key":item.key,"modifiers":item.modifiers,"submenu":matches!(row.action,Action::Group)}))
        }).collect::<Vec<_>>())
    }
    pub fn invoke_native_menu(&self, ctx: &Context, target: &str) -> bool {
        self.native_rows(ctx)
            .iter()
            .find(|r| r.target == target && r.item.enabled && !matches!(r.action, Action::Group))
            .is_some_and(|r| macos_menu::invoke(r.item.id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_key_equivalents_cover_function_navigation_and_letters() {
        for (key, expected) in [
            (Key::F1, '\u{f704}'),
            (Key::F8, '\u{f70b}'),
            (Key::F12, '\u{f70f}'),
            (Key::Home, '\u{f729}'),
            (Key::Delete, '\u{f728}'),
            (Key::ArrowLeft, '\u{f702}'),
            (Key::A, 'a'),
        ] {
            assert_eq!(equivalent(key), Some(expected.to_string()));
        }
    }
}
