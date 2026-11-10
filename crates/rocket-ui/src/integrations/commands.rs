//! Commands extensions declare: which rows they get in menus, right-click menus and the
//! palette, and what choosing one asks for. The rows come from the approved manifest, so
//! opening a menu runs no Lua and shows nothing a review hasn't.
use super::*;
use rocket_integrations::{Command, Target};

/// What keeps an extension's commands from running, and the row that clears it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fix {
    Review,
    Connect,
    Share,
    /// A saved draft belongs to other files or settings; its page offers Clear draft.
    Draft,
}
impl Fix {
    pub(crate) fn label(self, name: &str) -> String {
        match self {
            Self::Review => crate::i18n::trf!("Review {name}", name),
            Self::Connect => crate::i18n::trf!("Connect {name}", name),
            Self::Share => crate::i18n::trf!("Share repository details with {name}", name),
            Self::Draft => crate::i18n::trf!("Check {name}’s saved draft", name),
        }
    }
    /// Why a command waits, beside it in the palette.
    pub(crate) fn reason(self) -> &'static str {
        match self {
            Self::Review => crate::i18n::tr("Review its files first"),
            Self::Connect => crate::i18n::tr("Connect it first"),
            Self::Share => crate::i18n::tr("Needs repository details"),
            Self::Draft => crate::i18n::tr("Clear its saved draft first"),
        }
    }
}

/// What a command acts on, kept from the moment it was chosen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Chosen {
    Commit(String),
    Branch(String),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandTarget {
    pub repository: PathBuf,
    pub chosen: Chosen,
}
impl CommandTarget {
    pub(crate) fn commit(repository: &std::path::Path, id: &rocket_git::ObjectId) -> Self {
        Self { repository: repository.to_owned(), chosen: Chosen::Commit(id.as_str().to_owned()) }
    }
    /// A branch by its reference or short name.
    pub(crate) fn branch(repository: &std::path::Path, reference: &[u8]) -> Self {
        let short = short_ref(reference);
        let name: String = short.chars().filter(|c| !c.is_control()).take(1024).collect();
        Self { repository: repository.to_owned(), chosen: Chosen::Branch(name) }
    }
    pub(crate) fn kind(&self) -> Target {
        match self.chosen {
            Chosen::Commit(_) => Target::Commit,
            Chosen::Branch(_) => Target::Branch,
        }
    }
    pub(super) fn context(&self) -> Value {
        match &self.chosen {
            Chosen::Commit(id) => json!({"kind":"commit","commit":id}),
            Chosen::Branch(name) => json!({"kind":"branch","branch":name}),
        }
    }
}

/// A command somebody chose.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandRun {
    pub extension: String,
    pub command: String,
    pub label: String,
    pub target: Option<CommandTarget>,
}

impl Entry {
    pub(crate) fn commands_for(&self, target: Option<Target>) -> impl Iterator<Item = &Command> {
        self.commands.iter().filter(move |c| c.target == target)
    }
    /// What keeps commands on `target` from running, if anything does.
    pub(crate) fn fix_for(&self, target: Option<Target>) -> Option<Fix> {
        self.block.or((target.is_some() && !self.shares).then_some(Fix::Share))
    }
    pub(crate) fn run(&self, command: &Command, target: Option<CommandTarget>) -> CommandRun {
        CommandRun { extension: self.id.clone(), command: command.id.clone(), label: command.label.clone(), target }
    }
}

/// What the menus read while they draw. It lives on the egui context, like
/// `general::options`, so a right-click menu needs no path to the extensions.
#[derive(Clone, Default)]
pub(crate) struct Menu {
    pub(crate) entries: Arc<Vec<Entry>>,
    /// Why no command can start now.
    pub(crate) busy: Option<&'static str>,
}
fn menu_id() -> Id {
    Id::new("rocket-extension-commands")
}
pub(crate) fn menu(ctx: &Context) -> Menu {
    ctx.data(|d| d.get_temp(menu_id())).unwrap_or_default()
}
impl State {
    /// Publishes what menus show. `free` says the rest of Rocket leaves room to open a page.
    pub(crate) fn publish_commands(&self, ctx: &Context, free: bool) {
        let menu = Menu { entries: self.entries.clone(), busy: self.command_block(free) };
        ctx.data_mut(|d| d.insert_temp(menu_id(), menu));
    }
    /// Why a command can't start now: only one extension task runs at a time, and an open
    /// request review would be lost.
    pub(crate) fn command_block(&self, free: bool) -> Option<&'static str> {
        if !free || self.running() {
            Some(crate::i18n::tr("Wait for the current work to finish"))
        } else if self.preview.is_some() {
            Some(crate::i18n::tr("Finish the request review first"))
        } else {
            None
        }
    }
}

/// Which side of the extension rows a right-click menu's line goes.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Divider {
    Before,
    After,
}

/// One row per extension with commands for this target: a submenu of them, or the one row
/// that clears what is in their way.
pub(crate) fn context_rows(
    ui: &mut Ui,
    target: &CommandTarget,
    divider: Divider,
    intents: &mut Vec<Intent>,
    widgets: &mut HashMap<String, UiControl>,
) {
    let menu = menu(ui.ctx());
    let kind = Some(target.kind());
    let shown: Vec<&Entry> = menu.entries.iter().filter(|e| e.commands_for(kind).next().is_some()).collect();
    if shown.is_empty() {
        return;
    }
    if divider == Divider::Before {
        ui.separator();
    }
    let enabled = menu.busy.is_none();
    for entry in shown {
        if let Some(fix) = entry.fix_for(kind) {
            let r = ui.add_enabled(enabled, Button::new(fix.label(&entry.name)));
            pages::control(widgets, format!("ext-context-{}", entry.id), &r, ui.clip_rect());
            if r.clicked() {
                intents.push(Intent::ExtensionFix(entry.id.clone(), fix));
                ui.close();
            }
            continue;
        }
        ui.add_enabled_ui(enabled, |ui| {
            let row = ui.menu_button(entry.name.as_str(), |ui| {
                for command in entry.commands_for(kind) {
                    let r = ui.button(command_label(&command.label));
                    pages::control(widgets, format!("ext-context-{}-{}", entry.id, command.id), &r, ui.clip_rect());
                    if r.clicked() {
                        intents.push(Intent::ExtensionCommand(entry.run(command, Some(target.clone()))));
                        ui.close();
                    }
                }
            });
            pages::control(widgets, format!("ext-context-{}", entry.id), &row.response, ui.clip_rect());
        });
    }
    if divider == Divider::After {
        ui.separator();
    }
}

impl Rocket {
    /// A command from a menu: its extension's page, where it runs like a panel button.
    pub(crate) fn run_extension_command(&mut self, run: CommandRun) {
        let Some(backend) = self.backend.clone() else {
            return;
        };
        // A submenu can stay open after its row greys.
        if self.integrations.command_block(!self.interaction_busy()).is_some() {
            return;
        }
        self.integrations.playground.open = false;
        self.integrations.reveal_entry = true;
        self.integrations.perform(backend, Action::Command(run));
        self.dispatch(Intent::Dialog(DialogKind::Integrations));
    }
    pub(crate) fn fix_extension(&mut self, id: String, fix: Fix) {
        let Some(backend) = self.backend.clone() else {
            return;
        };
        match fix {
            Fix::Review => self.integrations.perform(backend, Action::Review(id)),
            Fix::Draft => self.open_extension(id),
            Fix::Connect | Fix::Share => {
                self.show_preferences(preferences::Page::Integrations);
                self.integrations.show_details(backend, id);
            }
        }
    }
}
