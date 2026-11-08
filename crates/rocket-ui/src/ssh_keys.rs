//! Account-specific SSH configuration, independent of hosting API tokens.
mod console;
mod discovery;
mod page;
use crate::{
    app::*,
    backend::user_error,
    pages::{self, control},
    style::*,
};
pub use console::Prompt;
use eframe::egui::*;
use rocket_git::*;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, mpsc},
};
enum Done {
    Inventory(SshInventory, Result<SshDiscovery>),
    /// A rescan, with ssh-agent's state when it could be read.
    Discovered(SshDiscovery, Option<(bool, Vec<String>)>),
    Created(SshProfile),
    Review(Arc<SshBindingReview>),
    Saved(PathBuf),
    Checked(String),
    /// A key's fingerprint and whether it is unlocked now.
    Agent(String, bool),
    Passphrase(SshProfile, bool),
}
pub struct State {
    pub enabled: bool,
    pub root: PathBuf,
    pub profiles: Vec<SshProfile>,
    /// The chosen key: a profile, or a found key without one (never both).
    pub selected: Option<usize>,
    pub found: Option<usize>,
    pub alias: String,
    pub host: String,
    pub user: String,
    pub port: String,
    pub existing: String,
    pub use_existing: bool,
    pub scan_directory: PathBuf,
    pub discovery: Option<SshDiscovery>,
    pub discovery_error: Option<String>,
    pub discovered_choice: Option<SshDiscoveredKey>,
    /// Review the repository setting once the key form's profile is created.
    pub review_after_create: bool,
    pub rsa: bool,
    pub creating: bool,
    pub loaded: bool,
    pub agent_running: bool,
    pub loaded_keys: Vec<String>,
    pub invalid: usize,
    pub error: Option<String>,
    pub notice: Option<String>,
    pub stopping: bool,
    pub operation: &'static str,
    pub preview: Option<Arc<SshBindingReview>>,
    pub prompt: Option<Prompt>,
    pub passphrase: String,
    pub authentication: SshAuthentication,
    task: Option<WriteTask<Done>>,
    input: Option<mpsc::SyncSender<String>>,
    prompts: Option<mpsc::Receiver<Prompt>>,
}
impl Default for State {
    fn default() -> Self {
        Self {
            enabled: true,
            root: std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
                .map(PathBuf::from)
                .unwrap_or_default()
                .join(".ssh/rocket-keys"),
            profiles: vec![],
            selected: None,
            found: None,
            alias: "rocket-personal".into(),
            host: "github.com".into(),
            user: "git".into(),
            port: "22".into(),
            existing: String::new(),
            use_existing: false,
            scan_directory: discovery::default_directory(),
            discovery: None,
            discovery_error: None,
            discovered_choice: None,
            review_after_create: false,
            rsa: false,
            creating: false,
            loaded: false,
            agent_running: false,
            loaded_keys: vec![],
            invalid: 0,
            error: None,
            notice: None,
            stopping: false,
            operation: "",
            preview: None,
            prompt: None,
            passphrase: String::new(),
            authentication: SshAuthentication::Agent,
            task: None,
            input: None,
            prompts: None,
        }
    }
}
impl State {
    fn clear_passphrase(&mut self) {
        let mut bytes = std::mem::take(&mut self.passphrase).into_bytes();
        bytes.fill(0);
        std::hint::black_box(&bytes);
    }
    pub fn running(&self) -> bool {
        self.task.is_some()
    }
    pub fn profile(&self) -> Option<&SshProfile> {
        self.selected.and_then(|i| self.profiles.get(i))
    }
    pub fn found_key(&self) -> Option<&SshDiscoveredKey> {
        self.found.and_then(|i| self.discovery.as_ref()?.keys.get(i))
    }
    pub fn cancel(&mut self) {
        if let Some(t) = &self.task {
            t.cancel();
            self.stopping = true;
            self.prompt = None;
            self.clear_passphrase();
            self.notice = Some(if self.operation == "ssh-discover" || self.operation == "ssh-list" {
                "Stopping the scan. The previous list will be retained."
            } else { "Stopping… waiting for OpenSSH, then checking the result." }.into())
        }
    }
}
impl Rocket {
    /// Preferences → SSH keys lists the profiles the first time it is shown.
    pub(crate) fn start_ssh_page(&mut self) {
        if self.preferences_page() == Some(crate::preferences::Page::SshKeys)
            && !self.ssh.loaded
            && !self.ssh.running()
            && self.ssh.enabled
        {
            self.ssh_action("list");
        }
    }
    pub(crate) fn poll_ssh(&mut self) {
        while let Some(p) = self.ssh.prompts.as_ref().and_then(|receiver| receiver.try_recv().ok()) {
            if !self.ssh.stopping {
                self.ssh.prompt = Some(p);
                self.ssh.clear_passphrase();
            }
        }
        let Some(result) = self.ssh.task.as_mut().and_then(WriteTask::try_finish) else {
            return;
        };
        let s = &mut self.ssh;
        s.task = None;
        s.input = None;
        s.prompts = None;
        s.prompt = None;
        s.clear_passphrase();
        let stopped = s.stopping;
        let reading = matches!(s.operation, "ssh-list" | "ssh-discover");
        // A failed or cancelled initial read is retried explicitly, never every frame.
        if s.operation == "ssh-list" { s.loaded = true; }
        s.stopping = false;
        s.operation = "";
        // "Use only this key" for a found key creates its profile, then reviews it.
        let mut review = false;
        match result {
            Ok(Done::Inventory(i,discovery))=>{s.profiles=i.profiles;s.agent_running=i.agent_running;s.loaded_keys=i.loaded;s.invalid=i.invalid;s.loaded=true;s.selected=s.selected.filter(|i|*i<s.profiles.len());s.receive_discovery(discovery);}
            Ok(Done::Discovered(d,agent))=>{if let Some((running,keys))=agent{s.agent_running=running;s.loaded_keys=keys;}s.receive_discovery(Ok(d));}
            Ok(Done::Created(p))=>{
                let mode=s.authentication;
                review=s.review_after_create && !(mode==SshAuthentication::Direct && p.encrypted && !p.passphrase_saved);
                s.profiles.push(p);s.select(page::Source::Profile(s.profiles.len()-1));s.authentication=mode;
                s.creating=false;s.discovered_choice=None;s.review_after_create=false;s.loaded=true;
                s.notice=Some(if mode==SshAuthentication::Direct&&!review {"Key profile ready. Save its passphrase for direct use, then review the repository setting."}else{"Key added. Copy its public key to your Git host."}.into());
            }
            Ok(Done::Review(r))=>{s.preview=Some(r);s.notice=None;}
            Ok(Done::Saved(path))=>{s.preview=None;s.notice=Some(format!("Repository SSH setting saved. Recovery backup: {}. Working files and remote URLs are unchanged.",path.display()));}
            Ok(Done::Checked(v))=>s.notice=Some(v),
            Ok(Done::Agent(fingerprint,unlocked))=>{
                s.loaded_keys.retain(|v|v!=&fingerprint);if unlocked{s.loaded_keys.push(fingerprint)}
                s.notice=Some(if stopped&&unlocked{"Stopped. OpenSSH had already unlocked this key; the result was checked. Untick it to remove it from ssh-agent."}
                    else if unlocked{if cfg!(windows) {"Key unlocked in Windows ssh-agent. Only this key was added."} else {"Key unlocked in ssh-agent. Only this key was added."}}
                    else if stopped{"Cancelled. The key is still locked."}else{"Key removed from ssh-agent. Key files and other unlocked keys are unchanged."}.into());
            }
            Ok(Done::Passphrase(p,saved))=>{
                if let Some(old)=s.profiles.iter_mut().find(|old|old.alias==p.alias){*old=p;}
                s.notice=Some(if !saved {"Saved passphrase forgotten. The private key, public key and agent are unchanged."}
                    else if cfg!(windows) {"Passphrase verified by OpenSSH and saved with Windows user encryption. Windows ssh-agent and key files were not changed."}
                    else if cfg!(target_os="macos") {"Passphrase verified by OpenSSH and saved, encrypted with a key in your macOS Keychain. The agent and key files were not changed."}
                    else {"Passphrase verified by OpenSSH and saved, encrypted with a key in your desktop keyring. ssh-agent and key files were not changed."}.into());
            }
            Err(Error::Cancelled)=>s.notice=Some(if reading {"Scan cancelled. The previous list is retained; keys and settings are unchanged."} else {"Cancelled. No profile or repository setting was published. Any incomplete key folder is retained for recovery."}.into()),
            Err(e)=>{s.error=Some(user_error(&e));s.preview=None;}
        }
        if review {
            self.ssh_action("review");
        }
    }
    fn ssh_action(&mut self, action: &'static str) {
        if self.ssh.running() {
            return;
        }
        let Some(backend) = self.backend.clone() else {
            return;
        };
        let repo = self
            .tab()
            .and_then(|t| t.snapshot.as_ref())
            .map(|s| s.repository.clone());
        let s = &mut self.ssh;
        s.error = None;
        s.notice = None;
        s.prompt = None;
        s.clear_passphrase();
        if !s.enabled {
            s.error=Some("SSH management is disabled in this MCP window. Use a marked disposable SSH fixture.".into());
            return;
        }
        let result = (|| -> Result<WriteTask<Done>> {
            let manager = SshManager::new(s.root.clone(), Some(backend.log.clone()))?;
            let profile = s.profile().cloned();
            let found = s.found_key().cloned();
            let op = match action {
                "list" => "ssh-list",
                "scan" => "ssh-discover",
                "create" => "ssh-create",
                "unlock" => "ssh-unlock",
                "unload" => "ssh-unload",
                "check" => "ssh-check",
                "review" | "restore" => "ssh-binding-review",
                "apply" => "ssh-binding-save",
                "remember" => "ssh-remember",
                "forget" => "ssh-forget",
                _ => return Err(Error::Ssh(SshFailure::InvalidProfile)),
            };
            s.operation = op;
            let alias = s.alias.clone();
            let host = s.host.clone();
            let user = s.user.clone();
            let port = if action == "create" { s.port.parse::<u16>().map_err(|_| Error::Ssh(SshFailure::InvalidProfile))? } else { 22 };
            let rsa = s.rsa;
            let existing = s.use_existing.then(|| PathBuf::from(&s.existing));
            let discovered = s.discovered_choice.clone();
            let scan_directory = s.scan_directory.clone();
            let review = s.preview.clone();
            let authentication = s.authentication;
            let (input, receive) = mpsc::sync_channel(1);
            let (prompts, prompt_receive) = mpsc::sync_channel(4);
            s.input = Some(input);
            s.prompts = Some(prompt_receive);
            WriteTask::spawn(move |cancel| {
                backend.log.request(op, |log| {
                    let manager = SshManager::new(manager.root().to_path_buf(), Some(log.clone()))?;
                    match action {
                        "list" => {
                            let inventory = manager.inventory(&cancel)?;
                            let discovery = manager.discover_keys(&scan_directory, &cancel);
                            cancel.check()?;
                            Ok(Done::Inventory(inventory, discovery))
                        }
                        "scan" => {
                            let discovery = manager.discover_keys(&scan_directory, &cancel)?;
                            // Rescan also shows what ssh-agent holds now.
                            let agent = manager.agent(&cancel).ok();
                            cancel.check()?;
                            Ok(Done::Discovered(discovery, agent))
                        }
                        "create" => {
                            if let Some(key) = discovered {
                                if existing.as_ref() != Some(&key.key) { return Err(Error::Ssh(SshFailure::Changed)); }
                                return manager.import_discovered(&key, &alias, &host, &user, port, &cancel).map(Done::Created);
                            }
                            let prepared = manager.prepare_with_algorithm(
                                &alias,
                                &host,
                                &user,
                                port,
                                existing.as_deref(),
                                rsa,
                                &cancel,
                            )?;
                            if existing.is_none() {
                                let code = console::console(
                                    &prepared.directory,
                                    "ssh-keygen.exe",
                                    prepared.arguments(),
                                    receive,
                                    prompts,
                                    &cancel,
                                    log,
                                )?;
                                cancel.check()?;
                                if code != Some(0) {
                                    return Err(Error::Ssh(SshFailure::ToolFailed));
                                }
                            }
                            prepared.finish(&cancel).map(Done::Created)
                        }
                        "remember" => {
                            let p = profile.ok_or(Error::Ssh(SshFailure::InvalidProfile))?;
                            let _guard = manager.guard(&p, &cancel)?;
                            prompts.try_send(Prompt::Remember).map_err(|_| Error::Cancelled)?;
                            let start = std::time::Instant::now();
                            let secret = loop {
                                cancel.check()?;
                                match receive.recv_timeout(std::time::Duration::from_millis(25)) {
                                    Ok(secret) => break secret,
                                    Err(mpsc::RecvTimeoutError::Disconnected) => return Err(Error::Cancelled),
                                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                                }
                                if start.elapsed() > std::time::Duration::from_secs(180) {
                                    return Err(Error::Cancelled);
                                }
                            };
                            manager.remember_passphrase(&p, secret, &cancel)
                                .map(|p| Done::Passphrase(p, true))
                        }
                        "forget" => manager.forget_passphrase(&profile.ok_or(Error::Ssh(SshFailure::InvalidProfile))?,&cancel).map(|p|Done::Passphrase(p,false)),
                        "unlock" => {
                            // A profile's key, or a found key while its files are the
                            // ones the scan read. Either stays held until ssh-add ends.
                            let (key, fingerprint, directory, _guard) = match (profile, found) {
                                (Some(p), _) => {
                                    let guard = manager.guard(&p, &cancel)?;
                                    (p.key, p.fingerprint, manager.root().to_path_buf(), guard)
                                }
                                (None, Some(k)) => {
                                    let guard = manager.guard_discovered(&k, &cancel)?;
                                    let directory = k
                                        .key
                                        .parent()
                                        .map(Path::to_path_buf)
                                        .ok_or(Error::Ssh(SshFailure::UnsafePath))?;
                                    (k.key, k.fingerprint, directory, guard)
                                }
                                (None, None) => return Err(Error::Ssh(SshFailure::InvalidProfile)),
                            };
                            let before = manager.agent(&cancel)?;
                            if !before.0 {
                                return Err(Error::Ssh(SshFailure::AgentUnavailable));
                            }
                            if before.1.contains(&fingerprint) {
                                return Ok(Done::Agent(fingerprint, true));
                            }
                            let key = key
                                .to_string_lossy()
                                .trim_start_matches(r"\\?\")
                                .to_owned();
                            let code = console::console(
                                &directory,
                                "ssh-add.exe",
                                vec!["-k".into(), key],
                                receive,
                                prompts,
                                &cancel,
                                log,
                            )?;
                            let (running, keys) = manager.agent(&CancellationToken::new())?;
                            if !running {
                                return Err(Error::Ssh(SshFailure::AgentUnavailable));
                            }
                            if keys.contains(&fingerprint) {
                                return Ok(Done::Agent(fingerprint, true));
                            }
                            if cancel.is_cancelled() {
                                return Ok(Done::Agent(fingerprint, false));
                            }
                            let _ = code;
                            Err(Error::Ssh(SshFailure::UnlockFailed))
                        }
                        "unload" => match (profile, found) {
                            (Some(p), _) => manager
                                .unload(&p, &cancel)
                                .map(|()| Done::Agent(p.fingerprint, false)),
                            (None, Some(k)) => manager
                                .unload_discovered(&k, &cancel)
                                .map(|()| Done::Agent(k.fingerprint, false)),
                            (None, None) => Err(Error::Ssh(SshFailure::InvalidProfile)),
                        },
                        "check" => manager
                            .check_authentication(
                                &profile.ok_or(Error::Ssh(SshFailure::InvalidProfile))?,
                                authentication,
                                &cancel,
                            )
                            .map(Done::Checked),
                        "review" | "restore" => {
                            let p = if action == "restore" {
                                None
                            } else {
                                Some(profile.ok_or(Error::Ssh(SshFailure::InvalidProfile))?)
                            };
                            repo.ok_or(Error::WorktreeRequired)?
                                .with_log(log.clone())
                                .preview_ssh_authentication(&manager, p.as_ref(), authentication, &cancel)
                                .map(|r| Done::Review(Arc::new(r)))
                        }
                        "apply" => repo
                            .ok_or(Error::WorktreeRequired)?
                            .with_log(log.clone())
                            .apply_ssh_binding(
                                &manager,
                                review.as_deref().ok_or(Error::Ssh(SshFailure::Changed))?,
                                &cancel,
                            )
                            .map(Done::Saved),
                        _ => Err(Error::Ssh(SshFailure::InvalidProfile)),
                    }
                })
            })
        })();
        match result {
            Ok(task) => s.task = Some(task),
            Err(e) => {
                s.error = Some(user_error(&e));
                s.input = None;
                s.prompts = None;
                s.operation = "";
            }
        }
    }
}
fn btn(
    ui: &mut Ui,
    w: &mut HashMap<String, UiControl>,
    key: &str,
    label: &str,
    enabled: bool,
) -> bool {
    let r = ui.add_enabled(enabled, Button::new(label));
    control(w, key, &r, ui.clip_rect());
    r.clicked()
}
/// Like `btn`, drawn as the page's main action.
fn primary_btn(
    ui: &mut Ui,
    w: &mut HashMap<String, UiControl>,
    key: &str,
    label: &str,
    enabled: bool,
) -> bool {
    let r = pages::primary(ui, enabled, label);
    control(w, key, &r, ui.clip_rect());
    r.clicked()
}
fn field(
    ui: &mut Ui,
    w: &mut HashMap<String, UiControl>,
    key: &str,
    label: &str,
    value: &mut String,
    enabled: bool,
    password: bool,
) {
    ui.label(label);
    let r = ui.add_enabled(
        enabled,
        TextEdit::singleline(value)
            .password(password)
            .desired_width(f32::INFINITY),
    );
    control(w, key, &r, ui.clip_rect());
}
