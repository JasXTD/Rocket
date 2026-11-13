//! Native extension host: the panel page and its renderer. No script runs on the UI thread.
mod commands;
mod extensions;
mod panel;
mod review;
#[cfg(test)]
mod tests;
pub(crate) use commands::{Chosen, CommandRun, CommandTarget, Divider, Fix, context_rows, menu};
pub(crate) use extensions::{Entry, letter_tile};
pub(crate) use panel::{Panel, PanelEvent, render_node};

use crate::{
    app::{DialogKind, Intent, Rocket, UiControl},
    backend::{Backend, user_error as backend_user_error},
    pages, preferences,
    style::*,
};
use eframe::egui::*;
use rocket_git::{Error, IntegrationConnection, Result, WriteTask};
use rocket_integrations::{
    ApprovedCopy, Discovery, Node, Outcome, Package, PackageIssue, Request, profiles, storage,
};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

fn user_error(error: &Error) -> String {
    match error {
        Error::InvalidInput(message) => message.clone(),
        _ => backend_user_error(error),
    }
}
/// An error's words without the code automation and diagnostics read.
fn shown(error: &str) -> &str {
    error
        .split_once(": ")
        .filter(|(code, _)| code.starts_with("integration_"))
        .map_or(error, |(_, message)| message)
}

enum Done {
    Loaded(Discovery, Value, Option<Vec<u8>>),
    Saved(Value, Vec<u8>, Option<(String, String)>),
    DraftSaved(Value, Vec<u8>, u64),
    Connection(Value, Vec<u8>, String),
    Panel(Outcome),
    Submitted(Value, Vec<u8>, Option<String>),
    Validated(rocket_integrations::Validation),
    /// A chosen file was read: what the review shows, or why it can't be installed.
    Picked(PathBuf, std::result::Result<Package, String>),
    /// A review or removal finished: the new catalog and store, and the extension to show.
    Changed(Discovery, Value, Option<Vec<u8>>, Option<String>),
}
enum Action {
    None,
    Package(String),
    Connection(String),
    Reload(Option<&'static str>),
    Save,
    /// The Enable switch of an extension, and what it was set to.
    Switch(String, bool),
    Review(String),
    Update(String),
    Run(PanelEvent),
    Command(CommandRun),
    Submit,
    Acknowledge,
    New,
    Remove,
    Clear,
    Validate,
}
#[derive(Clone, Default, PartialEq)]
pub struct Draft {
    pub name: String,
    pub origin: String,
    pub auth: String,
    pub username: String,
    pub token: String,
    pub remember: bool,
    pub writes: bool,
    pub repository: bool,
    pub settings: serde_json::Map<String, Value>,
}
pub struct State {
    pub playground: crate::playground::State,
    pub directory: PathBuf,
    pub packages: Vec<Package>,
    pub package_issues: Vec<PackageIssue>,
    /// What ships inside Rocket, whether or not it is on disk yet.
    pub bundled: Vec<Package>,
    /// The enabled extensions, kept current for the sidebar, menus and palette.
    entries: Arc<Vec<Entry>>,
    pub selected: String,
    connection_id: String,
    pub draft: Draft,
    /// The connection as saved, so Save and Discard show only after an edit.
    baseline: Draft,
    /// Preferences shows the selected extension instead of the list.
    details: bool,
    /// The list has been read at least once, so placeholders can go.
    listed_once: bool,
    list_error: Option<String>,
    prompt: Option<review::Prompt>,
    /// Frames the open prompt has shown, up to the two it spends rising above Preferences.
    prompt_frames: u8,
    tools_open: bool,
    /// Update from file was chosen; the page opens the picker once it can.
    pick: bool,
    /// Open folder was chosen for a package that couldn't load.
    reveal: Option<PathBuf>,
    /// An extension was just opened, so the sidebar or rail should bring it into view.
    reveal_entry: bool,
    pub view: Option<Arc<Node>>,
    pub values: HashMap<String, Value>,
    pub script_state: Value,
    pub preview: Option<Request>,
    pub error: Option<String>,
    pub notice: Option<String>,
    pub stopping: bool,
    /// The command that is running, for the line above the panel.
    working: Option<String>,
    /// The service confirmed a reviewed request, and no callback has finished since.
    confirmed: bool,
    pub inspected: bool,
    pub refreshed: bool,
    /// Set by the native fixture host; never loaded from settings or scripts.
    pub fixture: Option<u16>,
    pub isolated: bool,
    store: Value,
    raw: Option<Vec<u8>>,
    loaded: bool,
    task: Option<WriteTask<Done>>,
    memory_tokens: HashMap<String, String>,
    dirty_since: Option<Instant>,
    last_edit: Instant,
    generation: u64,
    saving_draft: bool,
    draft_error: Option<String>,
    draft_blocked: bool,
    pending: Option<Action>,
    remove_confirm: bool,
    clear_confirm: bool,
    repository: Option<(PathBuf, Option<rocket_git::ObjectId>)>,
    developer_open: bool,
    validation: Option<rocket_integrations::Validation>,
    started: Option<(Instant, &'static str, String, String)>,
    history: Vec<(String, String, &'static str, String, u128)>,
}
impl State {
    pub fn new(directory: PathBuf) -> Self {
        Self {
            playground: crate::playground::State::default(),
            bundled: rocket_integrations::bundled(&directory.join("integrations")),
            entries: Arc::new(vec![]),
            directory,
            packages: vec![],
            package_issues: vec![],
            selected: String::new(),
            connection_id: "default".into(),
            draft: Draft::default(),
            baseline: Draft::default(),
            details: false,
            listed_once: false,
            list_error: None,
            prompt: None,
            prompt_frames: 0,
            tools_open: false,
            pick: false,
            reveal: None,
            reveal_entry: false,
            view: None,
            values: HashMap::new(),
            script_state: json!({}),
            preview: None,
            error: None,
            notice: None,
            stopping: false,
            working: None,
            confirmed: false,
            inspected: false,
            refreshed: false,
            fixture: None,
            isolated: false,
            store: profiles::empty(),
            raw: None,
            loaded: false,
            task: None,
            memory_tokens: HashMap::new(),
            dirty_since: None,
            last_edit: Instant::now(),
            generation: 0,
            saving_draft: false,
            draft_error: None,
            draft_blocked: false,
            pending: None,
            remove_confirm: false,
            clear_confirm: false,
            repository: None,
            developer_open: false,
            validation: None,
            started: None,
            history: vec![],
        }
    }
    pub fn running(&self) -> bool {
        self.task.is_some() || self.playground.running()
    }
    pub fn review_open(&self) -> bool {
        self.prompt.is_some()
    }
    pub fn repository_selection(&mut self, repository: Option<(PathBuf, Option<rocket_git::ObjectId>)>) {
        self.repository = repository;
    }
    pub fn text_control(&self, target: &str) -> bool {
        if self.playground.text_control(target) { return true; }
        if matches!(
            target,
            "int-server" | "int-username" | "int-token" | "int-connection-name"
        ) {
            return true;
        }
        if let Some(id) = target.strip_prefix("int-setting-") {
            return self
                .package()
                .is_some_and(|p| p.settings.iter().any(|s| s.id == id));
        }
        fn input(node: &Node, id: &str) -> bool {
            node.id.as_deref() == Some(id) && matches!(node.kind.as_str(), "input" | "multiline")
                || node.children.iter().any(|n| input(n, id))
        }
        target
            .strip_prefix("int-ui-")
            .is_some_and(|id| self.view.as_ref().is_some_and(|n| input(n, id)))
    }
    pub fn cancel(&mut self) {
        self.pending = None;
        if let Some(task) = &self.task {
            task.cancel();
            self.stopping = true;
        }
    }
    pub fn prepare_exit(&mut self) {
        self.playground.prepare_exit();
        self.pending = None;
        if !self.saving_draft {
            self.cancel();
        }
    }
    fn package(&self) -> Option<&Package> {
        self.packages.iter().chain(&self.bundled).find(|p| p.id == self.selected)
    }
    /// The enabled extensions, in the order they list.
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }
    fn refresh_entries(&mut self) {
        let entries = self
            .packages
            .iter()
            .filter(|p| profiles::extension_enabled(&self.store, &p.id))
            .map(|p| {
                let chosen = profiles::connection(&self.store, &p.id, profiles::selected(&self.store, &p.id));
                let review = profiles::needs_review(&self.store, p);
                let unconnected = p.network && chosen["origin"].as_str().unwrap_or("").is_empty();
                // What a review has seen, whatever the files on disk say by now.
                let approved = profiles::approved_copy(&self.store, &p.id).and_then(ApprovedCopy::read);
                let name = approved.as_ref().map_or_else(|| p.name.clone(), |copy| copy.name.clone());
                Entry {
                    id: p.id.clone(),
                    letter: extensions::letter(&name),
                    name,
                    attention: review || profiles::unchecked_write(&self.store, &p.id).is_some(),
                    commands: approved.map(|copy| copy.commands).unwrap_or_default(),
                    block: if review {
                        Some(Fix::Review)
                    } else if unconnected {
                        Some(Fix::Connect)
                    } else if profiles::restore(p, chosen).is_err() {
                        Some(Fix::Draft)
                    } else {
                        None
                    },
                    shares: p.repository && chosen["repository"] == true,
                }
            })
            .collect();
        self.entries = Arc::new(entries);
    }
    /// The extension shown on the page, if the page has one to show.
    pub fn open_entry(&self) -> Option<&Entry> {
        (!self.playground.open).then(|| self.entries.iter().find(|e| e.id == self.selected)).flatten()
    }
    /// The list, for a route that names Extensions rather than one extension.
    pub(crate) fn show_list(&mut self) {
        self.details = false;
    }
    fn profile(&self) -> &Value {
        profiles::connection(&self.store, &self.selected, &self.connection_id)
    }
    fn token_key(&self) -> String {
        format!("{}/{}", self.selected, self.connection_id)
    }
    pub fn unsaved(&self) -> bool {
        self.dirty_since.is_some()
    }
    pub fn draft_save_error(&self) -> Option<&str> {
        self.draft_error.as_deref().or(self.playground.save_error.as_deref())
    }
    pub fn exit_pending(&self) -> bool { self.unsaved() || self.playground.dirty() }
    fn changed(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.last_edit = Instant::now();
        self.dirty_since.get_or_insert(self.last_edit);
    }
    fn connections(&self) -> Vec<(String, String)> {
        self.store["profiles"][&self.selected]["connections"]
            .as_object()
            .map(|m| {
                m.iter()
                    .map(|(k, v)| {
                        (
                            k.clone(),
                            v["name"].as_str().unwrap_or("Default").to_owned(),
                        )
                    })
                    .collect()
            })
            .unwrap_or_else(|| vec![("default".into(), "Default".into())])
    }
    pub fn uncertain(&self) -> bool {
        self.profile()["uncertain"].as_bool().unwrap_or(false)
    }
    /// This connection may run: on, approved for these files, and set up.
    fn enabled(&self) -> bool {
        self.profile()["enabled"].as_bool().unwrap_or(false)
            && self
                .package()
                .is_some_and(|p| p.approved(&self.profile()["approved"]))
            && !self.needs_connection()
    }
    /// A network extension whose connection has no server yet.
    fn needs_connection(&self) -> bool {
        self.package().is_some_and(|p| p.network) && self.profile()["origin"].as_str().unwrap_or("").is_empty()
    }
    pub fn snapshot(&self) -> Value {
        let mut ids = vec![];
        if let Some(view) = &self.view {
            view.control_ids(&mut ids);
        }
        let mut snapshot = json!({"busy":self.running(),"stopping":self.stopping,"loaded":self.loaded,"selected":self.selected,"connection":self.connection_id,"connections":self.connections().iter().map(|(id,_)|id).collect::<Vec<_>>(),"package_errors":self.package_issues.iter().map(|i|i.error.split(':').next().unwrap_or("integration_files")).collect::<Vec<_>>(),"draft_pending":self.unsaved(),"draft_saving":self.saving_draft,"draft_blocked":self.draft_blocked,"draft_saved":self.profile().get("draft").is_some(),"draft_error":self.draft_error.as_ref().map(|e|e.split(':').next().unwrap_or("integration_store")),"enabled":self.enabled(),"uncertain":self.uncertain(),"refreshed":self.refreshed,"review":self.preview.as_ref().map(|r|json!({"method":r.method,"has_body":r.body.is_some()})),"error":self.error.as_ref().map(|e|e.split(':').next().unwrap_or("integration_error")),"has_view":self.view.is_some(),"widget_ids":ids,"packages":self.packages.iter().chain(self.bundled.iter().filter(|b|!self.installed(&b.id))).map(|p|json!({"id":p.id,"name":p.name,"version":p.version,"network":p.network,"write":p.writes,"repository":p.repository,"installed":self.installed(&p.id),"enabled":profiles::extension_enabled(&self.store,&p.id)})).collect::<Vec<_>>()});
        snapshot["repository_allowed"] = json!(self.profile()["repository"] == true);
        snapshot["playground"] = self.playground.snapshot();
        snapshot["developer_open"] = json!(self.developer_open);
        snapshot["details"] = json!(self.details);
        snapshot["entries"] = json!(self.entries.iter().map(|e| json!({"id":e.id,"attention":e.attention,"shares":e.shares,
            "block":e.block.map(|b| format!("{b:?}").to_lowercase()),
            "commands":e.commands.iter().map(|c| json!({"id":c.id,"label":c.label,"target":c.target.map(|t| t.key())})).collect::<Vec<_>>()})).collect::<Vec<_>>());
        snapshot["extension_enabled"] = json!(profiles::extension_enabled(&self.store, &self.selected));
        snapshot["needs_review"] = json!(self.package().is_some_and(|p| profiles::needs_review(&self.store, p)));
        snapshot["needs_connection"] = json!(self.needs_connection());
        snapshot["unsaved_form"] = json!(self.draft != self.baseline);
        snapshot["prompt"] = self.prompt_snapshot();
        snapshot["validation"] = self.validation.as_ref().map(|v| json!({"valid":v.syntax_valid,"line":v.line})).unwrap_or(Value::Null);
        snapshot["recent_actions"] = json!(self.history.iter().filter(|(p,c,_,_,_)| p == &self.selected && c == &self.connection_id).map(|(_,_,op,code,ms)| json!({"action":op,"code":code,"duration_ms":ms})).collect::<Vec<_>>());
        snapshot
    }
    fn select(&mut self, id: String) {
        self.validation = None;
        self.selected = id;
        self.connection_id = profiles::selected(&self.store, &self.selected).into();
        self.restore_panel();
    }
    fn restore_panel(&mut self) {
        self.view = None;
        self.values.clear();
        self.script_state = json!({});
        self.preview = None;
        self.error = None;
        self.notice = None;
        self.confirmed = false;
        self.inspected = false;
        self.refreshed = false;
        self.dirty_since = None;
        self.draft_error = None;
        self.draft_blocked = false;
        self.remove_confirm = false;
        self.clear_confirm = false;
        self.reset_draft();
        if let Some(package) = self.package() {
            match profiles::restore(package, self.profile()) {
                Ok(Some(draft)) => {
                    self.view = Some(Arc::new(draft.view));
                    self.values = draft.values;
                    self.script_state = draft.state;
                }
                Ok(None) => {}
                Err(e) => {
                    self.draft_blocked = true;
                    self.error = Some(user_error(&e));
                }
            }
        }
    }
    fn reset_draft(&mut self) {
        let p = self.profile();
        let mut draft = Draft {
            name: p["name"].as_str().unwrap_or("Default").into(),
            origin: p["origin"].as_str().unwrap_or("").into(),
            auth: p["auth"].as_str().unwrap_or("none").into(),
            username: p["username"].as_str().unwrap_or("").into(),
            token: p["token"].as_str().unwrap_or("").into(),
            remember: p["remember"].as_bool().unwrap_or(false),
            writes: p["writes"].as_bool().unwrap_or(false),
            repository: p["repository"].as_bool().unwrap_or(false),
            settings: p["settings"].as_object().cloned().unwrap_or_default(),
        };
        if let Some(token) = self.memory_tokens.get(&self.token_key()) {
            draft.token = token.clone();
        }
        if let Some(package) = self.package() {
            for setting in &package.settings {
                draft
                    .settings
                    .entry(setting.id.clone())
                    .or_insert_with(|| json!(setting.default));
            }
        }
        self.baseline = draft.clone();
        self.draft = draft;
    }
    fn start(
        &mut self,
        operation: &'static str,
        run: impl FnOnce(rocket_git::CancellationToken) -> Result<Done> + Send + 'static,
    ) {
        if self.running() {
            return;
        }
        self.error = None;
        self.notice = None;
        self.stopping = false;
        self.started = Some((Instant::now(), operation, self.selected.clone(), self.connection_id.clone()));
        match WriteTask::spawn(run) {
            Ok(task) => self.task = Some(task),
            Err(e) => self.error = Some(user_error(&e)),
        }
    }
    fn reload(&mut self, backend: Backend, example: Option<&'static str>) {
        // A failed read waits for an explicit retry; do not retry every UI frame.
        self.loaded = true;
        self.list_error = None;
        let directory = self.directory.clone();
        self.start("Reload packages", move |cancel| {
            backend.integration("integration-discover", |_| {
                cancel.check()?;
                if let Some(example) = example {
                    rocket_integrations::install_example(&directory.join("integrations"), example)?;
                }
                let packages =
                    rocket_integrations::discover(&directory.join("integrations"), &cancel)?;
                for issue in &packages.issues {
                    let _: Result<()> = backend.log.request("integration-package", |_| {
                        Err(Error::InvalidInput(issue.error.clone()))
                    });
                }
                let (store, raw) = storage::load(&directory)?;
                Ok(Done::Loaded(packages, store, raw))
            })
        });
        if self.task.is_none() {
            self.listed_once = true;
            self.list_error = self.error.clone();
        }
    }
    /// Saves the selected connection. Only a review approves files: a save brings this
    /// connection along when the extension is on and current, and leaves it as it was otherwise.
    fn save(&mut self, backend: Backend) {
        let Some(package) = self.package().cloned() else {
            return;
        };
        if self.uncertain() {
            self.error=Some("integration_recovery: Inspect and acknowledge the previous remote outcome before changing this connection.".into());
            return;
        }
        let mut value = self.store.clone();
        let current = profiles::extension_enabled(&value, &package.id) && !profiles::needs_review(&value, &package);
        let saved = profiles::connection_mut(&mut value, &package.id, &self.connection_id);
        let draft = self.draft.clone();
        if let Err(e) = connection(&draft, self.fixture, package.network) {
            self.error = Some(user_error(&e));
            return;
        }
        let credential = match rocket_git::integration_credential_digest(&draft.token) {
            Ok(v) => v,
            Err(e) => {
                self.error = Some(user_error(&e));
                return;
            }
        };
        // Keep the identity of this connection stable. Its saved draft also
        // binds the exact origin/account/settings/credential, so replacement
        // blocks reuse but restoring the originals safely restores the draft.
        let revision = saved["revision"].as_str().unwrap_or("initial").to_owned();
        let old_draft = saved.get("draft").cloned();
        let enabled = current || saved["enabled"] == true;
        let approved = if current { Some(package.snapshot()) } else { saved.get("approved").cloned() };
        *saved = json!({"name":draft.name.trim(),"enabled":enabled,"origin":draft.origin,"auth":draft.auth,"username":draft.username,"token":if draft.remember{draft.token.as_str()}else{""},"credential":credential,"remember":draft.remember,"writes":draft.writes&&package.writes,"repository":draft.repository&&package.repository,"settings":draft.settings,"revision":revision,"uncertain":false});
        if let Some(approved) = approved {
            saved["approved"] = approved;
        }
        if let Some(mut draft) = old_draft {
            profiles::carry_draft(&mut draft, saved);
            saved["draft"] = draft;
        }
        if let Err(e) = profiles::validate(&value) {
            self.error = Some(user_error(&e));
            return;
        }
        let directory = self.directory.clone();
        let expected = self.raw.clone();
        // Commit the in-memory credential only alongside its successfully saved
        // origin/account. A failed save must keep the previous pair intact.
        let token = Some((self.token_key(), self.draft.token.clone()));
        self.start("Save settings", move |cancel| {
            backend.integration("integration-save", |_| {
                cancel.check()?;
                if current {
                    package.verify()?;
                }
                let raw = storage::save(&directory, &value, expected.as_deref())?;
                Ok(Done::Saved(value, raw, token))
            })
        });
    }
    /// The Enable switch. Files approved before turn on at once; anything else is reviewed.
    fn switch(&mut self, backend: Backend, id: &str, on: bool) {
        let Some(package) = self.packages.iter().chain(&self.bundled).find(|p| p.id == id).cloned() else {
            return;
        };
        if on && !(self.installed(id) && profiles::reviewed(&self.store, &package)) {
            self.ask_approval(id);
            return;
        }
        let mut value = self.store.clone();
        if on {
            profiles::approve(&mut value, &package);
        } else {
            profiles::disable(&mut value, id);
        }
        let directory = self.directory.clone();
        let expected = self.raw.clone();
        self.start("Save settings", move |cancel| {
            backend.integration("integration-save", |_| {
                cancel.check()?;
                if on {
                    package.verify()?;
                }
                let raw = storage::save(&directory, &value, expected.as_deref())?;
                Ok(Done::Saved(value, raw, None))
            })
        });
    }
    fn run(&mut self, backend: Backend, event: PanelEvent, command: Option<CommandRun>) {
        // The page says why, and offers Clear draft.
        if self.draft_blocked {
            return;
        }
        if !self.enabled() {
            self.error=Some("integration_disabled: Enable this extension and review its current files in Preferences first.".into());
            return;
        }
        if self.preview.is_some() {
            return;
        }
        let Some(package) = self.package().cloned() else {
            return;
        };
        if let Some(run) = &command {
            // The files may have changed since the row was drawn.
            let declared = package.commands.iter().find(|c| c.id == run.command).map(|c| c.target);
            if declared != Some(run.target.as_ref().map(CommandTarget::kind)) {
                self.error = Some("integration_command: This command is no longer part of the extension.".into());
                return;
            }
        }
        let profile = self.profile().clone();
        let Some(worker) = worker_path() else {
            self.error =
                Some("integration_worker: Cannot find Rocket's installation folder.".into());
            return;
        };
        let mut draft = Draft {
            origin: profile["origin"].as_str().unwrap_or("").into(),
            auth: profile["auth"].as_str().unwrap_or("none").into(),
            username: profile["username"].as_str().unwrap_or("").into(),
            token: profile["token"].as_str().unwrap_or("").into(),
            ..Draft::default()
        };
        if let Some(token) = self.memory_tokens.get(&self.token_key()) {
            draft.token = token.clone();
        }
        let conn = match connection(&draft, self.fixture, package.network) {
            Ok(c) => c,
            Err(e) => {
                self.error = Some(user_error(&e));
                return;
            }
        };
        let mut context = json!({"event":event.event,"settings":profile["settings"],"values":self.values,"state":self.script_state});
        if let Some(row) = &event.row {
            context["row"] = json!(row);
        }
        if self.confirmed {
            context["confirmed"] = json!(true);
        }
        let shares = package.repository && profile["repository"] == true;
        let mut repository = shares.then(|| self.repository.clone()).flatten();
        if let Some(run) = &command {
            context["command"] = json!(run.command);
            if let Some(target) = &run.target {
                if !shares {
                    self.error = Some("integration_permission: Share repository details with this extension first.".into());
                    return;
                }
                context["target"] = target.context();
                // The target's repository, which may no longer be the active one.
                if repository.as_ref().is_none_or(|(path, _)| *path != target.repository) {
                    repository = Some((target.repository.clone(), None));
                }
            }
        }
        let writes = profile["writes"].as_bool().unwrap_or(false) && !self.uncertain();
        let working = command.map(|run| run.label);
        let operation = if working.is_some() { "Run command" } else { "Run callback" };
        self.start(operation, move |cancel| {
            backend.integration("integration-run", |client| {
                if let Some((path, selected)) = repository {
                    context["repository"] = rocket_integrations::repository_context(client, &path, selected.as_ref(), &cancel)?;
                }
                rocket_integrations::execute(
                    client,
                    &worker,
                    &package,
                    context,
                    conn.as_ref(),
                    writes,
                    &cancel,
                )
                .map(Done::Panel)
            })
        });
        if self.running() {
            self.working = working;
        }
    }
    fn submit(&mut self, backend: Backend) {
        if !self.enabled() || self.uncertain() {
            return;
        }
        let (Some(package), Some(request)) = (self.package().cloned(), self.preview.clone()) else {
            return;
        };
        let profile = self.profile();
        if !profile["writes"].as_bool().unwrap_or(false) || !package.writes {
            return;
        }
        let mut draft = Draft {
            origin: profile["origin"].as_str().unwrap_or("").into(),
            auth: profile["auth"].as_str().unwrap_or("none").into(),
            username: profile["username"].as_str().unwrap_or("").into(),
            token: profile["token"].as_str().unwrap_or("").into(),
            ..Draft::default()
        };
        if let Some(token) = self.memory_tokens.get(&self.token_key()) {
            draft.token = token.clone();
        }
        let conn = match connection(&draft, self.fixture, true) {
            Ok(Some(c)) => c,
            Ok(None) => return,
            Err(e) => {
                self.error = Some(user_error(&e));
                return;
            }
        };
        let mut value = self.store.clone();
        let expected = self.raw.clone();
        let directory = self.directory.clone();
        let connection_id = self.connection_id.clone();
        self.start("Send reviewed request", move|cancel|backend.integration("integration-submit",|client|{
            cancel.check()?;package.verify()?;profiles::connection_mut(&mut value,&package.id,&connection_id)["uncertain"]=json!(true);
            let raw=storage::save(&directory,&value,expected.as_deref())?;
            let result=backend.log.request("integration-submit-result", |_| rocket_integrations::submit(client,&package,&conn,&request,&cancel));
            let unknown=result.as_ref().is_err_and(|e|matches!(e,Error::InvalidInput(s)if s.starts_with("integration_outcome_unknown:")));
            let error=result.err().map(|e|user_error(&e));if unknown{return Ok(Done::Submitted(value,raw,error));}
            profiles::connection_mut(&mut value,&package.id,&connection_id)["uncertain"]=json!(false);
            match storage::save(&directory,&value,Some(&raw)){Ok(next)=>Ok(Done::Submitted(value,next,error)),Err(_)=>{profiles::connection_mut(&mut value,&package.id,&connection_id)["uncertain"]=json!(true);Ok(Done::Submitted(value,raw,Some("integration_recovery: The request finished, but its recovery guard could not be cleared. Reload and inspect the service before retrying.".into())))}}
        }));
    }
    fn acknowledge(&mut self, backend: Backend) {
        if !self.inspected || !self.uncertain() {
            return;
        }
        let mut value = self.store.clone();
        profiles::connection_mut(&mut value, &self.selected, &self.connection_id)["uncertain"] =
            json!(false);
        let directory = self.directory.clone();
        let expected = self.raw.clone();
        self.start("Acknowledge outcome", move |cancel| {
            backend.integration("integration-save", |_| {
                cancel.check()?;
                let raw = storage::save(&directory, &value, expected.as_deref())?;
                Ok(Done::Saved(value, raw, None))
            })
        });
    }
    fn perform(&mut self, backend: Backend, action: Action) {
        if self.running() {
            return;
        }
        // Explicit draft removal is allowed even when the unsaved data exceeds a
        // limit. Other actions must not abandon an unsaved draft after I/O failure.
        if self.unsaved() && !matches!(action, Action::Clear) {
            self.persist_draft(backend, action);
            return;
        }
        match action {
            Action::None => {}
            // Back to the extension already open: a request review waiting there stays.
            Action::Package(id) if id == self.selected && self.preview.is_some() => {}
            Action::Package(id) => self.select(id),
            Action::Connection(id) => self.change_connection(backend, Some(id), false, false),
            Action::Reload(example) => self.reload(backend, example),
            Action::Save => self.save(backend),
            Action::Switch(id, on) => self.switch(backend, &id, on),
            Action::Review(id) => self.ask_approval(&id),
            Action::Update(id) => self.ask_update(&id),
            Action::Run(event) => self.run(backend, event, None),
            Action::Command(run) => {
                if self.selected != run.extension {
                    self.select(run.extension.clone());
                }
                self.run(backend, "command".into(), Some(run));
            }
            Action::Submit => self.submit(backend),
            Action::Acknowledge => self.acknowledge(backend),
            Action::New => self.change_connection(backend, None, false, false),
            Action::Remove => self.change_connection(backend, None, true, false),
            Action::Clear => {
                self.change_connection(backend, Some(self.connection_id.clone()), false, true)
            }
            Action::Validate => self.validate(backend),
        }
    }
    fn persist_draft(&mut self, backend: Backend, next: Action) {
        let (Some(package), Some(view)) = (self.package().cloned(), self.view.as_ref()) else {
            return;
        };
        let draft = match profiles::capture(
            &package,
            self.profile(),
            view,
            &self.values,
            &self.script_state,
        ) {
            Ok(v) => v,
            Err(e) => {
                self.draft_error = Some(user_error(&e));
                return;
            }
        };
        let mut value = self.store.clone();
        profiles::connection_mut(&mut value, &self.selected, &self.connection_id)["draft"] = draft;
        let expected = self.raw.clone();
        let directory = self.directory.clone();
        let generation = self.generation;
        self.pending = Some(next);
        self.draft_error = None;
        self.start("Save draft", move |cancel| {
            backend.integration("integration-draft-save", |_| {
                cancel.check()?;
                // Persisting old text is safe if source changed; restore/run checks
                // the exact source binding before giving any saved data to Lua.
                let raw = storage::save(&directory, &value, expected.as_deref())?;
                Ok(Done::DraftSaved(value, raw, generation))
            })
        });
        self.saving_draft = self.running();
        if !self.saving_draft {
            self.pending = None;
            self.draft_error = self.error.clone();
        }
    }
    fn change_connection(
        &mut self,
        backend: Backend,
        target: Option<String>,
        remove: bool,
        clear: bool,
    ) {
        if self.selected.is_empty() {
            return;
        }
        if remove && self.uncertain() {
            self.error = Some("integration_recovery: Inspect and acknowledge this connection's uncertain request before removing it.".into());
            return;
        }
        let mut value = self.store.clone();
        profiles::ensure(&mut value, &self.selected);
        let entry = &mut value["profiles"][&self.selected];
        let connections = entry["connections"].as_object_mut().unwrap();
        let id = if remove {
            let removed = connections.remove(&self.connection_id);
            if connections.is_empty() {
                // The extension stays on, with no connection yet, like one just enabled.
                let mut fresh = profiles::default_connection("Default");
                if let Some(removed) = removed.filter(|r| r["enabled"] == true) {
                    fresh["enabled"] = json!(true);
                    fresh["approved"] = removed["approved"].clone();
                }
                connections.insert("default".into(), fresh);
            }
            connections.keys().next().unwrap().clone()
        } else if let Some(id) = target {
            if !connections.contains_key(&id) {
                return;
            }
            if clear {
                connections
                    .get_mut(&id)
                    .unwrap()
                    .as_object_mut()
                    .unwrap()
                    .remove("draft");
            }
            id
        } else {
            if connections.len() >= 16 {
                self.error =
                    Some("integration_profile: Use at most 16 connections per extension.".into());
                return;
            }
            let id = unique_id();
            let mut number = connections.len() + 1;
            while connections.values().any(|p| {
                p["name"]
                    .as_str()
                    .is_some_and(|n| n.eq_ignore_ascii_case(&format!("Connection {number}")))
            }) {
                number += 1;
            }
            let mut connection = profiles::default_connection(&format!("Connection {number}"));
            connection["revision"] = json!(id);
            connections.insert(id.clone(), connection);
            id
        };
        entry["selected"] = json!(id);
        let directory = self.directory.clone();
        let expected = self.raw.clone();
        self.start("Change connection", move |cancel| {
            backend.integration("integration-profile-save", |_| {
                cancel.check()?;
                let raw = storage::save(&directory, &value, expected.as_deref())?;
                Ok(Done::Connection(value, raw, id))
            })
        });
    }
    fn validate(&mut self, backend: Backend) {
        let Some(package) = self.package() else { return; };
        let directory = package.directory.clone();
        let Some(worker) = worker_path() else { return; };
        self.validation = None;
        self.start("Validate Lua", move |cancel| {
            backend.integration("integration-validate", |client| {
                let validation = rocket_integrations::validate_package(client, &worker, &directory, &cancel)?;
                if !validation.syntax_valid {
                    let _: Result<()> = backend.log.request("integration-validation-result", |_| {
                        Err(Error::InvalidInput("integration_lua_syntax: Syntax validation failed.".into()))
                    });
                }
                Ok(Done::Validated(validation))
            })
        });
    }
    /// Adopts a fresh catalog and store, keeping session tokens only where their connection
    /// is unchanged.
    fn adopt(&mut self, discovery: Discovery, store: Value, raw: Option<Vec<u8>>) {
        self.packages = discovery.packages;
        self.package_issues = discovery.issues;
        // A reload can adopt an externally changed protected profile.
        // Session-only tokens belong to the original connection too.
        self.memory_tokens.retain(|id, _| {
            let Some((package, connection)) = id.split_once('/') else {
                return false;
            };
            [
                "origin",
                "auth",
                "username",
                "token",
                "remember",
                "credential",
                "revision",
            ]
            .iter()
            .all(|key| {
                profiles::connection(&self.store, package, connection)[*key]
                    == profiles::connection(&store, package, connection)[*key]
            })
        });
        self.store = store;
        self.raw = raw;
        self.loaded = true;
        self.listed_once = true;
    }
    pub fn poll(&mut self, ctx: &Context, backend: Option<Backend>, closing: bool) {
        self.playground.poll(ctx,backend.clone(),&self.directory,closing);
        // The sidebar lists enabled extensions, so the catalog loads without opening a page.
        // Reading it runs no script.
        if !self.loaded && !closing && !self.running()
            && let Some(backend) = backend.clone()
        {
            self.reload(backend, None);
        }
        if let Some(result) = self.task.as_mut().and_then(WriteTask::try_finish) {
            let listing = self.started.as_ref().is_some_and(|s| s.1 == "Reload packages");
            if let Some((start, operation, package, connection)) = self.started.take() {
                let error = match &result {
                    Err(e) => Some(user_error(e)),
                    Ok(Done::Submitted(_, _, Some(e))) => Some(e.clone()),
                    Ok(Done::Validated(v)) if !v.syntax_valid => Some("integration_lua_syntax:".into()),
                    _ => None,
                };
                let code = if matches!(result, Err(Error::Cancelled)) { "cancelled".to_owned() }
                    else { error.as_deref().and_then(|s| s.split_once(':').map(|(c,_)| c))
                        .filter(|c| c.len() <= 80 && c.bytes().all(|b| b.is_ascii_lowercase() || b == b'_'))
                        .unwrap_or(if error.is_some() {"failed"} else {"completed"}).to_owned() };
                if self.history.len() >= 32 { self.history.remove(0); }
                self.history.push((package, connection, operation, code, start.elapsed().as_millis()));
            }
            let saving = self.saving_draft;
            self.saving_draft = false;
            self.task = None;
            self.stopping = false;
            self.working = None;
            match result {
                Ok(Done::Validated(validation)) => {
                    self.notice = Some(if validation.syntax_valid {
                        crate::i18n::tr("Lua syntax is valid. No module initialization, callback, Git read or HTTP request was run. Reload folders to adopt edited files.").into()
                    } else {
                        crate::i18n::trf!("Lua syntax error in main.lua{line}. Check this line and its preceding block. Source and drafts are unchanged.", line = validation.line.map(|n| format!(":{n}")).unwrap_or_default())
                    });
                    self.validation = Some(validation);
                }
                Ok(Done::Loaded(discovery, store, raw)) => {
                    self.adopt(discovery, store, raw);
                    let selected = self
                        .packages
                        .iter()
                        .chain(&self.bundled)
                        .find(|p| p.id == self.selected)
                        .or_else(|| self.packages.first())
                        .map(|p| p.id.clone())
                        .unwrap_or_default();
                    self.select(selected);
                }
                Ok(Done::Picked(file, package)) => self.picked(file, package),
                Ok(Done::Changed(discovery, store, raw, show)) => {
                    self.adopt(discovery, store, raw);
                    self.prompt = None;
                    self.details = show.is_some();
                    let selected = show
                        .filter(|id| self.installed(id))
                        .or_else(|| self.packages.iter().find(|p| p.id == self.selected).map(|p| p.id.clone()))
                        .or_else(|| self.packages.first().map(|p| p.id.clone()))
                        .unwrap_or_default();
                    self.select(selected);
                }
                Ok(Done::Saved(store, raw, token)) => {
                    self.store = store;
                    self.raw = Some(raw);
                    if let Some((id, token)) = token {
                        self.memory_tokens.insert(id, token);
                    }
                    self.restore_panel();
                }
                Ok(Done::Connection(store, raw, id)) => {
                    self.memory_tokens.retain(|key, _| {
                        key.split_once('/').is_some_and(|(p, c)| {
                            !profiles::connection(&store, p, c).is_null()
                                && [
                                    "origin",
                                    "auth",
                                    "username",
                                    "token",
                                    "remember",
                                    "credential",
                                    "revision",
                                ]
                                .iter()
                                .all(|k| {
                                    profiles::connection(&self.store, p, c)[*k]
                                        == profiles::connection(&store, p, c)[*k]
                                })
                        })
                    });
                    self.store = store;
                    self.raw = Some(raw);
                    self.connection_id = id;
                    self.restore_panel();
                }
                Ok(Done::DraftSaved(store, raw, generation)) => {
                    self.store = store;
                    self.raw = Some(raw);
                    self.draft_error = None;
                    if generation == self.generation {
                        self.dirty_since = None;
                    }
                    if let (Some(action), Some(backend)) = (self.pending.take(), backend.clone())
                        && !matches!(action, Action::None)
                    {
                        self.perform(backend, action);
                    }
                }
                Ok(Done::Panel(Outcome::View { node, state })) => {
                    // Dynamic pages may replace control IDs. Retain only current
                    // fields so old values cannot grow without bound or leak into
                    // a later, unrelated form. Explicit cross-page data uses state.
                    self.values.retain(|id, value| node.accepts_value(id, value));
                    node.sync_segments(&mut self.values);
                    self.view = Some(Arc::from(node));
                    self.script_state = state;
                    self.changed();
                    self.preview = None;
                    self.confirmed = false;
                    self.refreshed = true;
                }
                Ok(Done::Panel(Outcome::Review(request))) => {
                    self.preview = Some(request);
                }
                Ok(Done::Submitted(store, raw, error)) => {
                    self.store = store;
                    self.raw = Some(raw);
                    self.preview = None;
                    self.inspected = false;
                    self.refreshed = false;
                    self.error = error;
                    if self.error.is_none() {
                        self.notice=Some(crate::i18n::tr("The service confirmed the request. Refresh the panel to inspect the result.").into());
                        self.confirmed = true;
                    }
                }
                Err(Error::Cancelled) => {
                    self.pending = None;
                    if saving {
                        self.draft_error = Some("integration_draft_cancelled: Draft save stopped. Your unsaved text is still here; retry before closing.".into());
                    }
                    self.notice = Some(crate::i18n::tr("Cancelled. The previous panel is preserved.").into())
                }
                Err(e) => {
                    self.pending = None;
                    if listing {
                        self.listed_once = true;
                        self.list_error = Some(user_error(&e));
                    }
                    if saving {
                        self.draft_error = Some(user_error(&e));
                    } else {
                        self.error = Some(user_error(&e));
                    }
                }
            }
            self.refresh_entries();
        }
        if let Some(since) = self.dirty_since
            && !self.running()
            && self.draft_error.is_none()
            && let Some(backend) = backend
        {
            if closing
                || self.last_edit.elapsed() >= Duration::from_millis(400)
                || since.elapsed() >= Duration::from_secs(2)
            {
                self.persist_draft(backend, Action::None);
            } else {
                ctx.request_repaint_after(Duration::from_millis(100));
            }
        }
        if self.running() {
            ctx.request_repaint_after(std::time::Duration::from_millis(25));
        }
    }
}
fn unique_id() -> String {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let time = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!(
        "c{time:x}-{:x}-{:x}",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    )
}
pub(crate) fn worker_path() -> Option<rocket_git::IntegrationWorker> {
    rocket_git::IntegrationWorker::for_host_executable(
        &std::env::current_exe().ok()?,
        env!("ROCKET_LUA_WORKER_SHA256"),
    )
}
fn basic(s: &str) -> String {
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for c in s.as_bytes().chunks(3) {
        let n = ((c[0] as u32) << 16)
            | ((c.get(1).copied().unwrap_or(0) as u32) << 8)
            | c.get(2).copied().unwrap_or(0) as u32;
        for shift in [18, 12] {
            out.push(T[((n >> shift) & 63) as usize] as char)
        }
        out.push(if c.len() > 1 {
            T[((n >> 6) & 63) as usize] as char
        } else {
            '='
        });
        out.push(if c.len() > 2 {
            T[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}
fn connection(
    draft: &Draft,
    fixture: Option<u16>,
    network: bool,
) -> Result<Option<IntegrationConnection>> {
    if !network {
        return Ok(None);
    }
    let invalid = |s: &str| Error::InvalidInput(s.into());
    if !rocket_integrations::text(&draft.username, 320)
        || draft.username.contains(':')
        || draft.username.chars().any(char::is_control)
        || draft.token.len() > 4096
        || draft.token.chars().any(char::is_control)
    {
        return Err(invalid(
            "integration_auth: Invalid account identifier or token.",
        ));
    }
    let authorization = match draft.auth.as_str() {
        "none" => String::new(),
        "bearer" if !draft.token.is_empty() => format!("Bearer {}", draft.token),
        "basic" if !draft.token.is_empty() && !draft.username.is_empty() => format!(
            "Basic {}",
            basic(&format!("{}:{}", draft.username, draft.token))
        ),
        _ => {
            return Err(invalid(
                "integration_auth: Choose authentication and enter its required credentials.",
            ));
        }
    };
    // Validate the user's visible origin even when a fixture replaces transport.
    let normal = IntegrationConnection::new(&draft.origin, &authorization)?;
    Ok(Some(if let Some(port) = fixture {
        IntegrationConnection::loopback_fixture(port, &authorization)?
    } else {
        normal
    }))
}
fn button(
    ui: &mut Ui,
    widgets: &mut HashMap<String, UiControl>,
    id: &str,
    label: &str,
    enabled: bool,
) -> bool {
    let r = ui.add_enabled(enabled, Button::new(label));
    pages::control(widgets, id, &r, ui.clip_rect());
    r.clicked()
}
/// A single-line field filling its row; the hint shows while it is empty.
fn edit(
    ui: &mut Ui,
    widgets: &mut HashMap<String, UiControl>,
    id: &str,
    value: &mut String,
    password: bool,
    hint: &str,
) {
    let r = ui.add_sized(
        [ui.available_width(), CONTROL],
        TextEdit::singleline(value)
            .id_salt(id)
            .password(password)
            .hint_text(hint)
            .align(Align2::LEFT_CENTER)
            .char_limit(4096),
    );
    pages::control(widgets, id, &r, ui.clip_rect());
}
/// A secondary action drawn without a frame until it is hovered.
fn quiet(
    ui: &mut Ui,
    widgets: &mut HashMap<String, UiControl>,
    id: &str,
    label: &str,
    enabled: bool,
) -> bool {
    let r = pages::ghost(ui, enabled, label);
    pages::control(widgets, id, &r, ui.clip_rect());
    r.clicked()
}

/// A centered message where a panel would be, with one main button.
fn panel_state(ui: &mut Ui, widgets: &mut HashMap<String, UiControl>, image: Icon, text: &str, action: Option<(&str, &str, bool)>) -> bool {
    let mut clicked = false;
    ui.add_space(40.0);
    ui.vertical_centered(|ui| {
        let (rect, _) = ui.allocate_exact_size(vec2(24.0, 24.0), Sense::hover());
        icon(ui.painter(), rect, image, FAINT());
        ui.add_space(8.0);
        ui.label(RichText::new(text).color(MUTED()));
        if let Some((id, label, enabled)) = action {
            ui.add_space(8.0);
            let r = pages::primary(ui, enabled, label);
            pages::control(widgets, id, &r, ui.clip_rect());
            clicked = r.clicked();
        }
    });
    clicked
}

impl Rocket {
    /// An extension's page: a header like other pages, its connection and Refresh, then its
    /// native panel, or the request review that replaces it before a write.
    pub(crate) fn integration_page(&mut self, ui: &mut Ui) {
        let Some(backend) = self.backend.clone() else {
            ui.label(crate::i18n::tr("Waiting for Rocket to start."));
            return;
        };
        let writable=self.profile_storage_ready();
        let s = &mut self.integrations;
        let widgets = &mut self.frame_widgets;
        if s.playground.open {
            crate::playground::view(ui,&mut s.playground,backend,s.directory.clone(),writable,widgets);
            if !s.playground.open {
                self.show_preferences(preferences::Page::Integrations);
            }
            return;
        }
        let idle = !s.running();
        let mut configure = false;
        let mut manage = false;
        let package = s.package().cloned().filter(|p| s.installed(&p.id) && profiles::extension_enabled(&s.store, &p.id));
        let Some(package) = package else {
            pages::header(ui, Icon::Puzzle, crate::i18n::tr("Extensions"), crate::i18n::tr("None open"), |ui| {
                manage = button(ui, widgets, "int-configure", crate::i18n::tr("Manage extensions"), true);
            });
            notices(ui, s, widgets, &backend, true);
            if working(s) {
                ui.horizontal(|ui| progress(ui, s, widgets));
            }
            if panel_state(ui, widgets, Icon::Puzzle, crate::i18n::tr("This extension isn't enabled"), None) || manage {
                self.show_preferences(preferences::Page::Integrations);
            }
            return;
        };
        let name = s.profile()["name"].as_str().unwrap_or("Default").to_owned();
        let origin = s.profile()["origin"].as_str().unwrap_or("").trim_start_matches("https://").to_owned();
        let detail = if !package.network {
            crate::i18n::trf!("{version} · Offline", version = package.version)
        } else if origin.is_empty() {
            crate::i18n::tr("No connection yet").to_owned()
        } else {
            format!("{origin} · {name}")
        };
        let letter = extensions::letter(&package.name);
        pages::header_with(
            ui,
            |painter, tile| {
                painter.text(tile.center(), Align2::CENTER_CENTER, letter, semibold(18.0), BLUE());
            },
            &package.name,
            &detail,
            |ui| configure = button(ui, widgets, "int-configure", crate::i18n::tr("Configure"), idle),
        );
        let review = profiles::needs_review(&s.store, &package);
        let wide = s.preview.is_none() && s.view.as_ref().is_some_and(|view| view.has_split());
        pages::toolbar(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            if s.connections().len() > 1 {
                choose_connection(ui, s, widgets, idle && s.preview.is_none(), &backend, true);
            }
            if button(
                ui,
                widgets,
                "int-run",
                crate::i18n::tr("Refresh panel"),
                idle && s.enabled() && s.preview.is_none() && !s.draft_blocked,
            ) {
                s.perform(backend.clone(), Action::Run("open".into()));
            }
            // In this row, so that work starting and ending never moves the panel.
            if working(s) {
                progress(ui, s, widgets);
            } else if (s.profile().get("draft").is_some() || s.unsaved()) && !s.draft_blocked {
                ui.label(RichText::new(if s.unsaved() {
                    crate::i18n::tr("Draft changes waiting to save")
                } else {
                    crate::i18n::tr("Draft saved on this device")
                }).size(12.0).color(HINT()));
                if quiet(ui, widgets, "int-clear-draft", crate::i18n::tr("Clear draft"), idle && s.preview.is_none()) {
                    s.clear_confirm = true;
                }
            }
        });
        let mut add_connection = false;
        let output = ScrollArea::vertical()
            .id_salt("integration-panel")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                Frame::new()
                    .inner_margin(Margin { left: 24, right: 24, top: 8, bottom: 24 })
                    .show(ui, |ui| {
                        ui.set_width(if wide { ui.available_width() } else { ui.available_width().min(960.0) });
                        ui.spacing_mut().item_spacing.y = 10.0;
                        notices(ui, s, widgets, &backend, true);
                        if review {
                            attention(ui, &crate::i18n::trf!("{extension}’s files changed. Review them before it can run.", extension = package.name), |ui| {
                                if button(ui, widgets, "int-review", crate::i18n::tr("Review"), idle) {
                                    s.perform(backend.clone(), Action::Review(package.id.clone()));
                                }
                            });
                        }
                        if s.draft_blocked {
                            attention(ui, crate::i18n::tr("This draft was saved with other files or connection settings."), |ui| {
                                if button(ui, widgets, "int-clear-draft", crate::i18n::tr("Clear draft"), idle && s.preview.is_none()) {
                                    s.clear_confirm = true;
                                }
                            });
                        }
                        if s.clear_confirm {
                            pages::callout(ui, AMBER(), crate::i18n::tr("Clear only this connection's saved form and script state? Other connections and remote data stay unchanged."));
                            ui.horizontal_wrapped(|ui| {
                                ui.spacing_mut().item_spacing.x = 8.0;
                                if button(ui, widgets, "int-clear-back", crate::i18n::tr("Keep draft"), idle) {
                                    s.clear_confirm = false;
                                }
                                let r = pages::danger(ui, idle, crate::i18n::tr("Clear this draft"));
                                pages::control(widgets, "int-clear-confirm", &r, ui.clip_rect());
                                if r.clicked() {
                                    s.perform(backend.clone(), Action::Clear);
                                }
                            });
                        }
                        if s.uncertain() {
                            pages::titled_card(
                                ui,
                                |ui| {
                                    ui.label(RichText::new(crate::i18n::tr("Check the previous remote result")).font(semibold(13.0)).color(AMBER()));
                                },
                                |ui| {
                                    pages::note(ui, crate::i18n::tr("A request may have reached the service. Check it there and refresh before sending another write. Restarting Rocket keeps this guard."));
                                    let r = ui.add(checkbox(&mut s.inspected, crate::i18n::tr("I checked the remote outcome; allow a new reviewed request")));
                                    pages::control(widgets, "int-inspected", &r, ui.clip_rect());
                                    if button(ui, widgets, "int-acknowledge", crate::i18n::tr("Acknowledge remote outcome"), idle && s.inspected) {
                                        s.perform(backend.clone(), Action::Acknowledge);
                                    }
                                },
                            );
                        }
                        if let Some(request) = s.preview.clone() {
                            review_card(ui, s, widgets, &backend, &request, idle);
                        } else if let Some(node) = s.view.clone() {
                            let mut action = None;
                            let enabled = (idle || s.saving_draft) && s.enabled() && !s.draft_blocked;
                            let mut changed = false;
                            let mut panel = Panel::new(&mut s.values, widgets, &mut action, &mut changed, "int-ui-").showing(&node);
                            render_node(ui, &node, &mut panel, enabled);
                            if changed {
                                s.changed();
                            }
                            if let Some(action) = action {
                                // Controls remain editable during a disk save. Button actions
                                // wait for that save before taking their exact input snapshot.
                                if s.saving_draft {
                                    s.pending = Some(Action::Run(action));
                                } else {
                                    s.perform(backend.clone(), Action::Run(action));
                                }
                            }
                        } else if s.needs_connection() {
                            let text = crate::i18n::trf!("Connect {extension} to see its panel", extension = package.name);
                            add_connection = panel_state(ui, widgets, Icon::Remote, &text, Some(("int-add-connection", crate::i18n::tr("Add connection"), idle)));
                        } else if !s.running() && !review && !s.draft_blocked {
                            let enabled = idle && s.enabled();
                            if panel_state(ui, widgets, Icon::Refresh, crate::i18n::tr("Not loaded yet"), Some(("int-run-first", crate::i18n::tr("Refresh panel"), enabled))) {
                                s.perform(backend.clone(), Action::Run("open".into()));
                            }
                        }
                    });
            });
        pages::region(
            widgets,
            "int-panel-scroll",
            output.inner_rect.intersect(ui.clip_rect()),
            true,
        );
        if configure || add_connection {
            self.configure_extension();
        }
    }
}
/// A line that holds the panel back: amber text and the one button that clears it.
fn attention(ui: &mut Ui, text: &str, action: impl FnOnce(&mut Ui)) {
    Frame::new()
        .fill(AMBER().gamma_multiply(0.08))
        .stroke(Stroke::new(1.0, AMBER().gamma_multiply(0.3)))
        .corner_radius(8)
        .inner_margin(Margin::symmetric(12, 7))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.allocate_ui_with_layout(vec2(ui.available_width(), CONTROL), Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                action(ui);
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    ui.add(Label::new(RichText::new(text).size(12.5).color(AMBER())).wrap());
                });
            });
        });
}
/// The exact request a write will send: method and address, the body with line
/// numbers, then Back or Send.
fn review_card(
    ui: &mut Ui,
    s: &mut State,
    widgets: &mut HashMap<String, UiControl>,
    backend: &Backend,
    request: &Request,
    idle: bool,
) {
    let address = format!("{}{}", s.profile()["origin"].as_str().unwrap_or(""), request.path);
    pages::titled_card(
        ui,
        |ui| {
            ui.label(RichText::new(crate::i18n::tr("Review the request")).font(semibold(13.0)));
            ui.label(RichText::new(crate::i18n::tr("Nothing has been sent")).size(12.0).color(HINT()));
        },
        |ui| {
            ui.spacing_mut().item_spacing.y = 8.0;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                pages::pill(ui, &request.method, VIOLET());
                ui.add(Label::new(RichText::new(&address).monospace().size(12.5)).truncate()).on_hover_text(&address);
            });
            pages::note(ui, crate::i18n::tr("Sent once, with this connection's credentials."));
            if let Some(check) = &request.check {
                pages::key_value(ui, crate::i18n::tr("Checks first"), 96.0, |ui| {
                    ui.label(RichText::new(crate::i18n::trf!("GET {path} · {pointer} must be {expected}", path = check.path, pointer = check.pointer, expected = check.expected)).size(12.5));
                });
                pages::note(ui, crate::i18n::tr("A changed marker stops the write and keeps your draft."));
            }
            let body = request
                .body
                .as_ref()
                .and_then(|v| serde_json::to_string_pretty(v).ok())
                .unwrap_or_else(|| crate::i18n::tr("No request body").into());
            let scroll = Frame::new()
                .fill(BG())
                .stroke(Stroke::new(1.0, HAIRLINE()))
                .corner_radius(8)
                .inner_margin(Margin::symmetric(4, 8))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ScrollArea::vertical()
                        .id_salt("integration-review")
                        .max_height(260.0)
                        .auto_shrink([false, true])
                        .show(ui, |ui| {
                            ui.spacing_mut().item_spacing.y = 2.0;
                            for (n, line) in body.lines().enumerate() {
                                ui.horizontal_top(|ui| {
                                    ui.spacing_mut().item_spacing.x = 10.0;
                                    let (rect, _) = ui.allocate_exact_size(vec2(30.0, 16.0), Sense::hover());
                                    ui.painter().text(rect.right_center(), Align2::RIGHT_CENTER, (n + 1).to_string(), FontId::monospace(12.0), FAINT());
                                    ui.add(Label::new(RichText::new(line).monospace().size(12.5)).wrap());
                                });
                            }
                        })
                })
                .inner;
            pages::region(
                widgets,
                "int-review-scroll",
                scroll.inner_rect.intersect(ui.clip_rect()),
                true,
            );
            pages::callout(ui, AMBER(), crate::i18n::tr("The service applies a sent write right away. Rocket can't undo it."));
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                if button(ui, widgets, "int-preview-back", crate::i18n::tr("Back"), idle) {
                    s.preview = None;
                    s.notice = Some(crate::i18n::tr("Review dismissed. No request was sent.").into());
                }
                let r = pages::primary(ui, idle && s.enabled() && !s.uncertain(), crate::i18n::tr("Send request"));
                pages::control(widgets, "int-preview-submit", &r, ui.clip_rect());
                if r.clicked() {
                    s.perform(backend.clone(), Action::Submit);
                }
            });
        },
    );
}
fn developer_panel(ui: &mut Ui, s: &mut State, widgets: &mut HashMap<String, UiControl>, backend: &Backend) {
    let r = pages::disclosure(ui, s.developer_open, crate::i18n::tr("Developer tools"));
    pages::control(widgets, "int-developer", &r, ui.clip_rect());
    if r.clicked() { s.developer_open = !s.developer_open; }
    if !s.developer_open { return; }
    let idle = !s.running();
    preferences::card_frame(ui, |ui| {
        preferences::card_header(ui, crate::i18n::tr("Developer"), |ui| {
            if button(ui, widgets, "int-validate", crate::i18n::tr("Validate Lua"), idle && s.preview.is_none() && s.installed(&s.selected)) {
                s.perform(backend.clone(), Action::Validate);
            }
            if quiet(ui, widgets, "int-history-clear", crate::i18n::tr("Clear"), idle) { s.history.clear(); }
        });
        preferences::card_note(ui, |ui| {
            if let Some(v) = &s.validation {
                let (color, text) = if v.syntax_valid {
                    (GREEN(), crate::i18n::tr("Syntax valid. Validation does not run the script.").to_owned())
                } else {
                    (RED(), crate::i18n::trf!("Syntax error · main.lua{line}", line = v.line.map(|n| format!(":{n}")).unwrap_or_default()))
                };
                preferences::status_line(ui, color, &text);
            }
            let scroll = ScrollArea::vertical().id_salt("integration-diagnostics").max_height(140.0).show(ui, |ui| {
                let mut none = true;
                for (package, connection, operation, code, ms) in s.history.iter().rev() {
                    if package == &s.selected && connection == &s.connection_id {
                        none = false;
                        ui.horizontal_wrapped(|ui| {
                            ui.label(RichText::new(*operation).size(12.5));
                            ui.label(RichText::new(code).monospace().size(12.0).color(MUTED()));
                            ui.label(RichText::new(format!("{ms} ms")).size(12.0).color(HINT()));
                        });
                    }
                }
                if none {
                    ui.label(RichText::new(crate::i18n::tr("No recent actions")).size(12.5).color(HINT()));
                }
            });
            pages::region(widgets, "int-developer-scroll", scroll.inner_rect.intersect(ui.clip_rect()), true);
        });
    });
}
fn choose_connection(
    ui: &mut Ui,
    s: &mut State,
    widgets: &mut HashMap<String, UiControl>,
    enabled: bool,
    backend: &Backend,
    labelled: bool,
) {
    if s.package().is_none() {
        return;
    }
    let current = s.profile()["name"].as_str().unwrap_or("Default").to_owned();
    let mut choose = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        if labelled {
            ui.label(RichText::new(crate::i18n::tr("Connection")).color(MUTED()));
        }
        ui.add_enabled_ui(enabled, |ui| {
            let response = ComboBox::from_id_salt("integration-connection")
                .selected_text(current)
                .width(210.0)
                .show_ui(ui, |ui| {
                    for (id, name) in s.connections() {
                        let r = ui.selectable_label(id == s.connection_id, name);
                        pages::control(widgets, format!("int-connection-{id}"), &r, ui.clip_rect());
                        if r.clicked() {
                            choose = Some(id);
                        }
                    }
                });
            pages::control(
                widgets,
                "int-connection",
                &response.response,
                ui.clip_rect(),
            );
        });
    });
    if let Some(id) = choose
        && id != s.connection_id
    {
        s.perform(backend.clone(), Action::Connection(id));
    }
}
/// Whether work is running that should show. A draft saves in a moment, after every result,
/// so only a slow save does.
fn working(s: &State) -> bool {
    let saving = s.started.as_ref().filter(|_| s.saving_draft && !s.stopping);
    s.running() && saving.is_none_or(|(start, ..)| start.elapsed() >= Duration::from_secs(1))
}
fn progress(ui: &mut Ui, s: &mut State, widgets: &mut HashMap<String, UiControl>) {
    ui.spacing_mut().item_spacing.x = 8.0;
    ui.spinner();
    ui.label(RichText::new(if s.stopping {
        crate::i18n::tr("Stopping the worker and checking the result")
    } else if s.saving_draft {
        crate::i18n::tr("Saving protected draft")
    } else {
        s.working.as_deref().unwrap_or(crate::i18n::tr("Running"))
    }).color(MUTED()));
    if button(ui, widgets, "int-cancel", crate::i18n::tr("Stop"), !s.stopping) {
        s.cancel();
    }
}
/// Errors, and outside a panel work in progress. `panel` also shows the panel's status line,
/// and leaves out a blocked draft's error, which has its own line with Clear draft.
fn notices(
    ui: &mut Ui,
    s: &mut State,
    widgets: &mut HashMap<String, UiControl>,
    backend: &Backend,
    panel: bool,
) {
    if let Some(error) = &s.error
        && !(panel && s.draft_blocked && error.starts_with("integration_draft_changed"))
        && s.prompt.is_none()
    {
        pages::callout(ui, RED(), shown(error));
    }
    if panel && let Some(notice) = &s.notice {
        pages::note(ui, notice);
    }
    if let Some(error) = s.draft_error.clone() {
        pages::callout(ui, RED(), &crate::i18n::trf!("{error} Your unsaved draft remains in this window; existing disk data was preserved.", error = shown(&error)));
        if button(
            ui,
            widgets,
            "int-draft-retry",
            crate::i18n::tr("Retry saving draft"),
            !s.running(),
        ) {
            s.perform(backend.clone(), Action::None);
        }
    }
    if !panel && working(s) {
        ui.horizontal(|ui| progress(ui, s, widgets));
    }
}
