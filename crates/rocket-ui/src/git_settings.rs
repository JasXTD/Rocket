//! Git Preferences. Probes and config writes never run on the drawing thread.
use crate::{
    i18n::{tr, trf},
    app::Rocket,
    backend::user_error,
    pages,
    preferences::{card_foot, card_frame, card_header, card_note, status_line, text_row},
    style::*,
    views::display_path,
};
use eframe::egui::*;
use rocket_git::*;
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};

const FILE: &str = "git-selection";
const MANAGED: &str = "managed";
#[derive(Clone, Debug, Default, PartialEq, Eq)]
enum Choice {
    #[default]
    Automatic,
    Managed,
    Path(PathBuf),
}
impl Choice {
    fn executable(&self) -> Result<Option<PathBuf>> {
        match self {
            Self::Automatic => Ok(None),
            Self::Managed => managed_git_path().map(Some),
            Self::Path(path) => Ok(Some(path.clone())),
        }
    }
    fn encode(&self) -> Result<Vec<u8>> {
        let data = match self {
            Self::Managed => "ROCKET-GIT-2\nmanaged".to_owned(),
            Self::Automatic => "ROCKET-GIT-1\n".to_owned(),
            Self::Path(path) => format!("ROCKET-GIT-1\n{}", path.to_str()
                .ok_or_else(|| Error::InvalidInput("Git selection must be valid Unicode".into()))?),
        };
        decode(Some(data.as_bytes()))?;
        Ok(data.into_bytes())
    }
}
fn io(action: &'static str, source: std::io::Error) -> Error {
    Error::Io { action, source }
}
fn read_file(path: &Path) -> Result<Option<Vec<u8>>> {
    match fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(io("read Git selection", e)),
        Ok(meta) => {
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                if meta.file_attributes() & 0x400 != 0 {
                    return Err(Error::InvalidInput(
                        "Git selection must be a normal file".into(),
                    ));
                }
            }
            if !meta.is_file() || meta.len() > 32768 {
                return Err(Error::InvalidInput(
                    "Invalid Git selection file; preserve it and choose Automatic again".into(),
                ));
            }
        }
    }
    let mut data = Vec::new();
    fs::File::open(path)
        .and_then(|f| f.take(32769).read_to_end(&mut data))
        .map_err(|e| io("read Git selection", e))?;
    if data.len() > 32768 {
        return Err(Error::InvalidInput("Git selection is too large".into()));
    }
    Ok(Some(data))
}
fn decode(data: Option<&[u8]>) -> Result<Choice> {
    let Some(data) = data else { return Ok(Choice::Automatic) };
    if data == b"ROCKET-GIT-2\nmanaged" { return Ok(Choice::Managed); }
    let value=std::str::from_utf8(data).ok().and_then(|s|s.strip_prefix("ROCKET-GIT-1\n"))
        .ok_or_else(||Error::InvalidInput("Unsupported Git selection. Choose Automatic or a checked executable in Preferences".into()))?;
    if value.is_empty() {
        return Ok(Choice::Automatic);
    }
    let path = PathBuf::from(value);
    if !path.is_absolute() || value.chars().any(char::is_control) {
        return Err(Error::InvalidInput(
            "Saved Git executable must be an absolute path".into(),
        ));
    }
    Ok(Choice::Path(path))
}
pub(crate) fn selection(directory: &Path) -> Result<Option<PathBuf>> {
    decode(read_file(&directory.join(FILE))?.as_deref())?.executable()
}
fn save_selection(
    directory: &Path,
    expected: Option<&[u8]>,
    selected: &Choice,
    cancel: &CancellationToken,
) -> Result<()> {
    cancel.check()?;
    fs::create_dir_all(directory).map_err(|e| io("create settings folder", e))?;
    let path = directory.join(FILE);
    let lock = directory.join("git-selection.lock");
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&lock)
        .map_err(|e| io("lock Git selection; another writer may be active", e))?;
    let result = (|| {
        if read_file(&path)?.as_deref() != expected {
            return Err(Error::InvalidInput(
                "Git selection changed externally; Reload before saving".into(),
            ));
        }
        let data = selected.encode()?;
        file.write_all(&data)
            .and_then(|_| file.sync_all())
            .map_err(|e| io("write Git selection", e))?;
        drop(file);
        cancel.check()?;
        if read_file(&path)?.as_deref() != expected {
            return Err(Error::InvalidInput("Git selection changed externally; Reload before saving".into()));
        }
        cancel.check()?;
        fs::rename(&lock, &path).map_err(|e| io("publish Git selection", e))
    })();
    if result.is_err() {
        let _ = fs::remove_file(&lock);
    }
    result
}

/// What `git --version` said about one copy of Git.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Check {
    /// Usable: its complete version, including the Windows security build.
    Ready(String),
    /// Local/SSH operations work, but the runtime cannot safely use HTTP(S).
    Restricted(Box<(String, &'static str)>),
    /// Older than Rocket supports: its complete version.
    Old(String),
    /// Not usable: why.
    Broken(String),
}
/// A copy of Git on this computer. `check` is `None` when it was not run: automation
/// runs only the Git this window already uses.
#[derive(Clone, Debug)]
struct Found {
    path: PathBuf,
    check: Option<Check>,
}
impl Found {
    fn usable(&self) -> bool {
        matches!(self.check, None | Some(Check::Ready(_) | Check::Restricted(_)))
    }
}

enum Finished {
    Loaded {
        bytes: Option<Vec<u8>>,
        selected: Choice,
        identity: Box<std::result::Result<GlobalIdentity, String>>,
        selection_error: Option<String>,
        automatic: Option<Found>,
        candidates: Vec<Found>,
        resolved: Option<PathBuf>,
        managed_available: bool,
        managed_verified: bool,
    },
    SelectionSaved {
        choice: Choice,
        bytes: Vec<u8>,
        executable: PathBuf,
        version: String,
    },
    IdentitySaved(std::result::Result<GlobalIdentity, String>),
    ManagedVerified,
}
/// What the running task does, so its result and messages land on the right card.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Job {
    #[default]
    Load,
    Choose,
    Identity,
    Verify,
}
#[derive(Default)]
pub struct State {
    pub loaded: bool,
    /// The choice shown: Automatic, or the program at `path`.
    pub automatic: bool,
    pub managed: bool,
    pub managed_available: bool,
    pub managed_verified: bool,
    pub managed_progress: ManagedGitProgress,
    pub path: String,
    /// The Custom path field; it keeps its text while another choice is shown.
    pub custom: String,
    pub name: String,
    pub email: String,
    pub identity: Option<GlobalIdentity>,
    pub error: Option<String>,
    pub notice: Option<String>,
    /// Whether `error` and `notice` are about the identity rather than the Git program.
    pub about_identity: bool,
    /// The identity was just saved; the card says so until the next edit or task.
    identity_saved: bool,
    /// The last choice saved here: the text saved, its executable and version.
    pub checked: Option<(String, PathBuf, String)>,
    automatic_found: Option<Found>,
    candidates: Vec<Found>,
    saved: Choice,
    /// The executable the saved choice runs, when known; Rocket switches on restart.
    resolved: Option<PathBuf>,
    /// The choice being checked, for its error message.
    trying: String,
    /// The Custom path field is open although another choice is saved.
    custom_open: bool,
    /// Browse chose a program for the Custom path: check and save it on the next frame.
    picked: bool,
    bytes: Option<Vec<u8>>,
    task: Option<WriteTask<Finished>>,
    job: Job,
    pub stopping: bool,
}
impl State {
    pub(crate) fn cancel(&mut self) {
        if let Some(task) = &self.task {
            self.stopping = true;
            task.cancel();
        }
    }
    pub fn running(&self) -> bool {
        self.task.is_some()
    }
    fn choice(&self) -> String {
        if self.automatic {
            String::new()
        } else if self.managed {
            MANAGED.to_owned()
        } else {
            self.path.trim().to_owned()
        }
    }
    /// The identity fields differ from the saved global name and email. A Git choice is
    /// saved as it is made, so it is never unsaved.
    pub fn dirty(&self) -> bool {
        self.loaded
            && self.identity.as_ref().is_some_and(|i| {
                self.name != i.direct.name.as_deref().unwrap_or("")
                    || self.email != i.direct.email.as_deref().unwrap_or("")
            })
    }
    /// The installed copies listed under Automatic: each one found except the copy
    /// Automatic runs, so no path shows twice.
    fn listed(&self) -> Vec<Found> {
        let automatic = self.automatic_found.as_ref().map(|f| &f.path);
        self.candidates
            .iter()
            .filter(|f| Some(&f.path) != automatic)
            .cloned()
            .collect()
    }
    /// The program a click on `target` checks and saves: "" for Automatic, a path for an
    /// installed copy or the Custom path; `None` when the click saves nothing.
    pub fn choice_for(&self, target: &str) -> Option<String> {
        let custom = self.custom.trim();
        match target {
            "git-automatic" => Some(String::new()),
            "git-managed" => Some(MANAGED.to_owned()),
            "git-custom" | "git-save" => (!custom.is_empty()).then(|| custom.to_owned()),
            _ => {
                let n: usize = target.strip_prefix("git-candidate-")?.parse().ok()?;
                self.listed().get(n).map(|f| display_path(&f.path))
            }
        }
    }
    /// Browse chose `path` for the Custom path; it is checked and saved like any choice.
    pub(crate) fn picked(&mut self, path: String) {
        self.custom = path;
        self.custom_open = true;
        self.picked = true;
    }
    /// Where Browse starts: the Custom path, else the saved program.
    pub(crate) fn browse_start(&self) -> String {
        if self.custom.trim().is_empty() {
            self.path.clone()
        } else {
            self.custom.clone()
        }
    }
    /// Preferences stays open while Git is checked or the identity is unsaved; say why.
    pub(crate) fn hold_close(&mut self) {
        self.about_identity = if self.running() {
            self.job == Job::Identity
        } else {
            true
        };
        self.error = None;
        self.notice = Some(
            if self.running() {
                tr("Wait for Git to finish, or stop it.")
            } else {
                tr("Save or discard the identity change first.")
            }
            .into(),
        );
    }
    /// Show the saved choice again after a check failed or was stopped.
    fn show_saved(&mut self) {
        self.automatic = self.saved == Choice::Automatic;
        self.managed = self.saved == Choice::Managed;
        self.path = self.saved.executable().ok().flatten()
            .map(|p| display_path(&p)).unwrap_or_default();
    }
    fn start(
        &mut self,
        job: Job,
        work: impl FnOnce(CancellationToken) -> Result<Finished> + Send + 'static,
    ) {
        self.error = None;
        self.notice = None;
        self.about_identity = job == Job::Identity;
        self.identity_saved = false;
        self.stopping = false;
        self.job = job;
        match WriteTask::spawn(work) {
            Ok(task) => self.task = Some(task),
            Err(e) => {
                if job == Job::Choose { self.show_saved(); }
                self.error = Some(user_error(&e));
            }
        }
    }
}
fn candidates() -> Vec<PathBuf> {
    let mut folders = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
        .unwrap_or_default();
    for (name, suffix) in [
        ("ProgramFiles", "Git/cmd"),
        ("ProgramFiles(x86)", "Git/cmd"),
        ("LOCALAPPDATA", "Programs/Git/cmd"),
    ] {
        if let Some(root) = std::env::var_os(name) {
            folders.push(PathBuf::from(root).join(suffix));
        }
    }
    let mut result = Vec::new();
    for folder in folders.into_iter().filter(|p| p.is_absolute()) {
        let path = folder.join(if cfg!(windows) { "git.exe" } else { "git" });
        if path.is_file()
            && let Ok(path) = path.canonicalize()
            && !result.contains(&path)
        {
            result.push(path)
        }
    }
    result
}
/// Share automatic resolution with the backend, including a damaged bundled copy
/// whose verification should produce a useful error rather than fall back to PATH.
fn automatic_executable() -> Option<PathBuf> {
    GitClient::automatic_executable().ok().map(|path| path.canonicalize().unwrap_or(path))
}
/// What `git --version` says about `path`, within a short time limit.
fn check(path: &Path, cancel: &CancellationToken) -> Check {
    let limits = CommandLimits {
        timeout: Duration::from_secs(5),
        ..Default::default()
    };
    match GitClient::discover_with_options(Some(path), limits, None, cancel) {
        Ok(git) => match git.http_transport_warning() {
            Some(warning) => Check::Restricted(Box::new((display_version(git.version()), warning))),
            None => Check::Ready(display_version(git.version())),
        },
        // "Git 2.46 or later required; found git version 2.45.2.windows.1"
        Err(Error::Unsupported(message)) => match message.split_once("found ") {
            Some((_, version)) => Check::Old(display_version(version)),
            None => Check::Broken(message),
        },
        Err(error) => Check::Broken(user_error(&error)),
    }
}
/// Keep vendor/build suffixes: they can distinguish vulnerable and fixed binaries.
fn display_version(text: &str) -> String {
    let text = text.trim();
    let version = text.strip_prefix("git version ").unwrap_or(text);
    version.to_owned()
}
impl Rocket {
    pub(crate) fn load_git_settings(&mut self) {
        if self.git_settings.running() {
            return;
        }
        let directory = self.state_directory.clone();
        let backend = self.backend.clone();
        // Automation runs only the Git this window already uses; other copies are listed
        // without a version.
        let only = self
            .browse_root
            .is_some()
            .then(|| backend.as_ref().map(|b| b.executable.clone()));
        self.git_settings.start(Job::Load, move |cancel| {
            let bytes = read_file(&directory.join(FILE))?;
            let (selected, selection_error) = match decode(bytes.as_deref()) {
                Ok(s) => (s, None),
                Err(e) => (Choice::Automatic, Some(user_error(&e))),
            };
            let identity = backend
                .as_ref()
                .ok_or_else(|| "Git is unavailable. Choose a Git that works, then restart Rocket.".to_owned())
                .and_then(|b| b.global_identity(&cancel).map_err(|e| user_error(&e)));
            cancel.check()?;
            let run = |path: &Path| match &only {
                Some(running) if running.as_deref() != Some(path) => None,
                _ => Some(check(path, &cancel)),
            };
            let candidates: Vec<Found> = candidates()
                .into_iter()
                .map(|path| Found {
                    check: run(&path),
                    path,
                })
                .collect();
            cancel.check()?;
            let automatic = automatic_executable().map(|path| {
                candidates
                    .iter()
                    .find(|f| f.path == path)
                    .cloned()
                    .unwrap_or_else(|| Found {
                        check: run(&path),
                        path,
                    })
            });
            let resolved = match selected.executable()? {
                Some(path) => path.canonicalize().ok(),
                None => automatic.as_ref().map(|f| f.path.clone()),
            };
            let managed_path = managed_git_path()?.canonicalize().ok();
            let managed_available = managed_path.as_ref().is_some_and(|p| p.is_file());
            let managed_verified = managed_path.as_ref().is_some_and(|p| backend.as_ref().is_some_and(|b| &b.executable == p));
            Ok(Finished::Loaded {
                bytes,
                selected,
                identity: Box::new(identity),
                selection_error,
                automatic,
                candidates,
                resolved,
                managed_available,
                managed_verified,
            })
        });
    }
    pub(crate) fn poll_git_settings(&mut self, ctx: &Context) {
        let s = &mut self.git_settings;
        if let Some(result) = s.task.as_mut().and_then(|t| t.try_finish()) {
            s.task = None;
            s.stopping = false;
            match result {
                Ok(Finished::Loaded {
                    bytes,
                    selected,
                    identity,
                    selection_error,
                    automatic,
                    candidates,
                    resolved,
                    managed_available,
                    managed_verified,
                }) => {
                    s.bytes = bytes;
                    s.saved = selected;
                    s.show_saved();
                    s.automatic_found = automatic;
                    s.candidates = candidates;
                    s.resolved = resolved;
                    s.managed_available = managed_available;
                    s.managed_verified = managed_verified;
                    // A saved program that is not an installed copy shows as the Custom path.
                    if !s.automatic && !s.managed && !s.listed().iter().any(|f| display_path(&f.path) == s.path) {
                        s.custom = s.path.clone();
                    }
                    s.loaded = true;
                    s.checked = None;
                    s.error = selection_error;
                    s.about_identity = false;
                    match *identity {
                        Ok(i) => {
                            s.name = i.direct.name.clone().unwrap_or_default();
                            s.email = i.direct.email.clone().unwrap_or_default();
                            s.identity = Some(i)
                        }
                        Err(e) => {
                            s.identity = None;
                            s.error = Some(e);
                            s.about_identity = true;
                        }
                    }
                }
                Ok(Finished::SelectionSaved {
                    choice,
                    bytes,
                    executable,
                    version,
                }) => {
                    s.saved = choice;
                    s.bytes = Some(bytes);
                    s.show_saved();
                    s.managed_verified |= s.managed;
                    s.checked = Some((s.choice(), executable.clone(), version));
                    s.resolved = Some(executable);
                }
                Ok(Finished::ManagedVerified) => {
                    s.managed_verified = true;
                    s.notice = Some(tr("Bundled Git files verified. Your Git choice is unchanged.").into());
                }
                Ok(Finished::IdentitySaved(refreshed)) => match refreshed {
                    Ok(i) => {
                        s.name = i.direct.name.clone().unwrap_or_default();
                        s.email = i.direct.email.clone().unwrap_or_default();
                        s.identity = Some(i);
                        s.identity_saved = true;
                    }
                    Err(e) => {
                        s.identity = None;
                        s.error = Some(trf!("Saved, but reading it back failed: {error}", error = e));
                    }
                },
                Err(e) => {
                    let stopped = matches!(e, Error::Cancelled);
                    match s.job {
                        // The saved choice stays: show it again.
                        Job::Choose => {
                            s.show_saved();
                            if stopped {
                                s.notice = Some(tr("Stopped. The Git choice is unchanged.").into());
                            } else {
                                s.error = Some(trf!("{choice} cannot be used: {error}", choice = s.trying, error = user_error(&e)));
                            }
                        }
                        Job::Load if stopped => s.notice = Some(tr("Stopped before Git was found.").into()),
                        Job::Verify => {
                            s.managed_verified = false;
                            if stopped { s.notice = Some(tr("Stopped. The Git choice is unchanged.").into()); }
                            else { s.error = Some(user_error(&e)); }
                        }
                        _ => s.error = Some(user_error(&e)),
                    }
                }
            }
            ctx.request_repaint();
        }
        if s.running() {
            ctx.request_repaint_after(Duration::from_millis(25))
        }
    }
    /// Check `choice` ("" for Automatic) and save it once it runs; Rocket switches to it on
    /// restart. A failed or stopped check leaves the saved choice.
    fn choose_git(&mut self, choice: String) {
        if self.git_settings.running() { return; }
        let directory = self.state_directory.clone();
        let log_path = self.log_path.clone();
        let s = &mut self.git_settings;
        if !choice.is_empty() && choice != MANAGED && !Path::new(&choice).is_absolute() {
            s.error = Some(tr("Use the full path to git.exe, starting with the drive.").into());
            s.notice = None;
            s.about_identity = false;
            return;
        }
        s.automatic = choice.is_empty();
        s.managed = choice == MANAGED;
        if !choice.is_empty() {
            s.path = choice.clone();
        }
        s.trying = if choice.is_empty() {
            tr("Automatic").into()
        } else if s.managed {
            tr("Bundled Git").into()
        } else {
            choice.clone()
        };
        let observed = s.bytes.clone();
        s.managed_progress = ManagedGitProgress::default();
        let progress = s.managed_progress.clone();
        s.start(Job::Choose, move |cancel| {
            let log = DiagnosticLog::open(&log_path)
                .map_err(|e| io("open Git settings diagnostics", e))?;
            log.request("git-selection-save", |log| {
                let selected = match choice.as_str() {
                    "" => Choice::Automatic,
                    MANAGED => Choice::Managed,
                    _ => Choice::Path(PathBuf::from(&choice)),
                };
                // Keep the verified lease alive through discovery and publication.
                let _lease = if selected == Choice::Managed {
                    Some(log.request("managed-git-verify", |_| verify_managed_git(&cancel, &progress))?)
                } else { None };
                let executable = selected.executable()?;
                let git = GitClient::discover_with_options(
                    executable.as_deref(),
                    CommandLimits {
                        timeout: Duration::from_secs(10),
                        ..Default::default()
                    },
                    Some(log.clone()),
                    &cancel,
                )?;
                let bytes = selected.encode()?;
                save_selection(&directory, observed.as_deref(), &selected, &cancel)?;
                Ok(Finished::SelectionSaved {
                    choice: selected,
                    bytes,
                    executable: git.executable().into(),
                    version: git.version().into(),
                })
            })
        });
    }
    fn verify_bundled_git(&mut self) {
        if self.git_settings.running() { return; }
        let log_path = self.log_path.clone();
        self.git_settings.managed_progress = ManagedGitProgress::default();
        let progress = self.git_settings.managed_progress.clone();
        self.git_settings.start(Job::Verify, move |cancel| {
            let log = DiagnosticLog::open(&log_path).map_err(|e| io("open Git settings diagnostics", e))?;
            log.request("managed-git-verify", |_| {
                let _lease = verify_managed_git(&cancel, &progress)?;
                Ok(Finished::ManagedVerified)
            })
        });
    }
    /// Preferences → Git: the Git program as one list of choices, each saved as it is
    /// chosen, then the global name and email with Save once they are edited. Returns
    /// whether Browse was clicked for the Custom path.
    pub(crate) fn git_settings_page(&mut self, ui: &mut Ui) -> bool {
        let s = &self.git_settings;
        if !s.loaded && !s.running() && s.error.is_none() && s.notice.is_none() {
            self.load_git_settings()
        }
        let running = self
            .backend
            .as_ref()
            .map(|b| (display_version(&b.version), b.executable.clone()));
        let writing = self.write.is_some();
        let busy = self.git_settings.running();
        let mut choose = std::mem::take(&mut self.git_settings.picked)
            .then(|| self.git_settings.custom.trim().to_owned())
            .filter(|c| !c.is_empty());
        let mut browse = false;
        let mut save_identity = false;
        let mut discard = false;
        let mut retry = false;
        let mut stop = false;
        let mut verify_managed = false;
        let s = &mut self.git_settings;
        let widgets = &mut self.frame_widgets;
        let job = s.job;
        let listed = s.listed();
        card_frame(ui, |ui| {
            card_header(ui, tr("Git program"), |ui| {
                if busy && job != Job::Identity {
                    let r = ui.add_enabled(
                        !s.stopping,
                        Button::new(if s.stopping { tr("Stopping") } else { tr("Stop") }),
                    );
                    pages::control(widgets, "git-cancel", &r, ui.clip_rect());
                    stop |= r.clicked();
                    ui.label(
                        RichText::new(if job == Job::Load { tr("Looking for Git") } else { tr("Checking Git") })
                            .size(12.0)
                            .color(HINT()),
                    );
                    ui.spinner();
                } else if let Some((version, _)) = &running {
                    pages::pill(ui, &trf!("Git {version} in use", version), GREEN());
                } else {
                    pages::pill(ui, tr("Git unavailable"), RED());
                }
            });
            if !s.loaded {
                // A read that failed or was stopped: say so where the list goes, with Try again.
                if !busy {
                    card_note(ui, |ui| {
                        if let Some(e) = s.error.as_ref().filter(|_| !s.about_identity) {
                            status_line(ui, RED(), e);
                        } else if let Some(n) = &s.notice {
                            status_line(ui, BLUE(), n);
                        }
                        let r = ui.button(tr("Try again"));
                        pages::control(widgets, "git-reload", &r, ui.clip_rect());
                        retry |= r.clicked();
                    });
                }
                return;
            }
            ui.add_enabled_ui(!busy, |ui| {
                let (detail, automatic_check) = match &s.automatic_found {
                    Some(f) => (
                        trf!("Bundled when included, otherwise installed · {path}", path = display_path(&f.path)),
                        f.check.clone(),
                    ),
                    None => (tr("No bundled or installed Git found").to_owned(), None),
                };
                let r = choice_row(ui, s.automatic, true, tr("Automatic"), &detail, true, |ui| {
                    check_pill(ui, automatic_check.as_ref())
                });
                pages::control(widgets, "git-automatic", &r, ui.clip_rect());
                if r.clicked() && !s.automatic {
                    choose = Some(String::new());
                    s.custom_open = false;
                }
                let detail = trf!("Git for Windows {version} · travels with Rocket", version = MANAGED_GIT_VERSION);
                let r = ui.add_enabled_ui(s.managed_available, |ui| {
                    choice_row(ui, s.managed, s.managed_available, tr("Bundled Git"), &detail, false, |ui| {
                        pages::pill(ui, if !s.managed_available { tr("Not included") } else if s.managed_verified { tr("Verified") } else { tr("Included") },
                            if s.managed_verified { GREEN() } else { MUTED() });
                    })
                }).inner;
                pages::control(widgets, "git-managed", &r, ui.clip_rect());
                if r.clicked() && s.managed_available && !s.managed {
                    choose = Some(MANAGED.to_owned());
                    s.custom_open = false;
                }
                card_note(ui, |ui| {
                    ui.label(RichText::new(tr("Pinned runtime. Files are checked before use; installed Git, PATH and your credentials stay unchanged.")).size(12.0).color(HINT()));
                    let r = ui.add_enabled(s.managed_available, Button::new(tr("Verify files")));
                    pages::control(widgets, "git-managed-verify", &r, ui.clip_rect());
                    verify_managed |= r.clicked();
                });
                for (n, found) in listed.iter().enumerate() {
                    let path = display_path(&found.path);
                    let selected = !s.automatic && !s.managed && s.path == path;
                    let r = choice_row(ui, selected, found.usable(), tr("Installed copy"), &path, true, |ui| {
                        check_pill(ui, found.check.as_ref())
                    });
                    pages::control(widgets, format!("git-candidate-{n}"), &r, ui.clip_rect());
                    if r.clicked() && found.usable() && !selected {
                        choose = Some(path);
                        s.custom_open = false;
                    }
                }
                let custom_saved =
                    !s.automatic && !s.managed && !listed.iter().any(|f| display_path(&f.path) == s.path);
                let detail = match s.custom.trim() {
                    "" => tr("Any git.exe on this computer").to_owned(),
                    path => path.to_owned(),
                };
                let path_given = !s.custom.trim().is_empty();
                let r = choice_row(ui, custom_saved, true, tr("Custom path"), &detail, path_given, |ui| {
                    let b = ui.button(tr("Browse"));
                    pages::control(widgets, "git-browse", &b, ui.clip_rect());
                    browse |= b.clicked();
                });
                pages::control(widgets, "git-custom", &r, ui.clip_rect());
                if r.clicked() && !custom_saved {
                    // A path given before is checked again; otherwise the field waits for one.
                    s.custom_open = true;
                    if !s.custom.trim().is_empty() {
                        choose = Some(s.custom.trim().to_owned());
                    }
                }
                if custom_saved || s.custom_open {
                    Frame::new()
                        .inner_margin(Margin { left: 46, right: 14, top: 2, bottom: 12 })
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 8.0;
                                let width = (ui.available_width() - 60.0).max(120.0);
                                let r = ui.add_sized(
                                    [width, CONTROL],
                                    TextEdit::singleline(&mut s.custom)
                                        .font(TextStyle::Monospace)
                                        .align(Align2::LEFT_CENTER)
                                        .hint_text(
                                            RichText::new(r"C:\Program Files\Git\cmd\git.exe")
                                                .monospace()
                                                .color(FAINT())
                                                .italics(),
                                        ),
                                );
                                pages::control(widgets, "git-program", &r, ui.clip_rect());
                                let enter = r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                                let typed = s.custom.trim();
                                let ready = !typed.is_empty() && !(custom_saved && typed == s.path.trim());
                                let r = pages::primary(ui, ready, tr("Use"));
                                pages::control(widgets, "git-save", &r, ui.clip_rect());
                                if ready && (r.clicked() || enter) {
                                    choose = Some(typed.to_owned());
                                }
                            });
                        });
                }
            });
            if busy && (job == Job::Verify || (job == Job::Choose && s.managed)) {
                let (checked, total, _, _) = s.managed_progress.snapshot();
                card_note(ui, |ui| {
                    let fraction = if total == 0 { 0.0 } else { checked as f32 / total as f32 };
                    ui.add(ProgressBar::new(fraction).text(if s.stopping { tr("Stopping verification").to_owned() }
                        else { trf!("Checking bundled Git · {checked} / {total} files", checked, total) }));
                });
            }
            // A choice that failed, and a saved choice that waits for a restart.
            let restart = running
                .as_ref()
                .zip(s.resolved.as_ref())
                .is_some_and(|((_, now), next)| now != next);
            let error = s.error.as_ref().filter(|_| !s.about_identity);
            let notice = s.notice.as_ref().filter(|_| !s.about_identity);
            if error.is_some() || notice.is_some() || (restart && !busy) {
                card_note(ui, |ui| {
                    if let Some(e) = error {
                        status_line(ui, RED(), e);
                        let r = ui.add_enabled(!busy && !s.dirty(), Button::new(tr("Reload")));
                        pages::control(widgets, "git-reload", &r, ui.clip_rect());
                        retry |= r.clicked();
                    }
                    if let Some(n) = notice {
                        status_line(ui, BLUE(), n);
                    }
                    if restart && !busy {
                        status_line(ui, BLUE(), tr("Saved. Restart Rocket to switch to this Git."));
                    }
                });
            }
        });
        let dirty = s.dirty();
        if dirty {
            s.identity_saved = false;
        }
        let saving = busy && job == Job::Identity;
        let value = identity_value(&s.name, &s.email);
        card_frame(ui, |ui| {
            card_header(ui, tr("Commit identity"), |ui| {
                if let Some(i) = &s.identity {
                    let mut about = trf!("Saved in {path}. A repository can set its own name and email.", path = display_path(&i.path));
                    if !i.origins.is_empty() {
                        about.push_str("\n\n");
                        about.push_str(&i.origins.join("\n"));
                    }
                    let (rect, r) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
                    icon(ui.painter(), rect, Icon::Info, FAINT());
                    r.on_hover_text(about);
                    ui.label(RichText::new(tr("Every repository")).size(12.0).color(HINT()));
                }
            });
            // An empty field shows the value Git uses, from included files too, as a hint.
            let hint = |value: Option<&String>| {
                RichText::new(value.map_or(tr("Not set"), String::as_str))
                    .color(FAINT())
                    .italics()
            };
            let (name_hint, email_hint) = s.identity.as_ref().map_or_else(
                || (hint(None), hint(None)),
                |i| (hint(i.effective.name.as_ref()), hint(i.effective.email.as_ref())),
            );
            Frame::new()
                .inner_margin(Margin::symmetric(0, 6))
                .show(ui, |ui| {
                    ui.add_enabled_ui(s.identity.is_some() && !busy, |ui| {
                        for (label, key, value, hint) in [
                            (tr("Name"), "git-name", &mut s.name, name_hint),
                            (tr("Email"), "git-email", &mut s.email, email_hint),
                        ] {
                            text_row(ui, label, 84.0, |ui| {
                                let r = ui.add_sized(
                                    [ui.available_width(), CONTROL],
                                    TextEdit::singleline(value)
                                        .char_limit(1024)
                                        .align(Align2::LEFT_CENTER)
                                        .hint_text(hint),
                                );
                                pages::control(widgets, key, &r, ui.clip_rect());
                            });
                        }
                    });
                });
            let invalid = value.validate().err().map(|e| user_error(&e));
            let error = s.error.as_ref().filter(|_| s.about_identity);
            let notice = s.notice.as_ref().filter(|_| s.about_identity);
            // Saved, unless an included config file overrides what was saved.
            let saved = s.identity.as_ref().filter(|_| s.identity_saved && !dirty).map(|i| {
                if i.direct == i.effective {
                    (GREEN(), tr("Saved for every repository."))
                } else {
                    (AMBER(), tr("Saved. An included config file still overrides part of it."))
                }
            });
            let retry_shown = s.identity.is_none() && !busy && s.loaded;
            if invalid.is_some() || error.is_some() || notice.is_some() || saved.is_some() || retry_shown {
                Frame::new()
                    .inner_margin(Margin { left: 14, right: 14, top: 0, bottom: 10 })
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.spacing_mut().item_spacing.y = 6.0;
                        if let Some(e) = &invalid {
                            status_line(ui, AMBER(), e);
                        }
                        if let Some(e) = error {
                            status_line(ui, RED(), e);
                        }
                        if let Some(n) = notice {
                            status_line(ui, AMBER(), n);
                        }
                        if let Some((color, text)) = saved {
                            status_line(ui, color, text);
                        }
                        if retry_shown {
                            let r = ui.button(tr("Try again"));
                            pages::control(widgets, "git-reload", &r, ui.clip_rect());
                            retry |= r.clicked();
                        }
                    });
            }
            // Save and Discard appear once a field is edited.
            if dirty || saving {
                card_foot(ui, Some(AMBER().gamma_multiply(0.05)), |ui| {
                    if saving {
                        let r = ui.add_enabled(
                            !s.stopping,
                            Button::new(if s.stopping { tr("Stopping") } else { tr("Stop") }),
                        );
                        pages::control(widgets, "git-cancel", &r, ui.clip_rect());
                        stop |= r.clicked();
                        ui.spinner();
                    } else {
                        let writable = s.identity.as_ref().is_some_and(|i| i.direct != value)
                            && value.validate().is_ok()
                            && !writing;
                        let r = pages::primary(ui, writable, tr("Save"));
                        pages::control(widgets, "git-identity-save", &r, ui.clip_rect());
                        save_identity = r.clicked();
                        let r = ui.button(tr("Discard"));
                        pages::control(widgets, "git-reload", &r, ui.clip_rect());
                        discard = r.clicked();
                    }
                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        if saving {
                            ui.label(RichText::new(tr("Saving")).size(12.5).color(HINT()));
                        } else {
                            let (dot, _) = ui.allocate_exact_size(vec2(7.0, 7.0), Sense::hover());
                            ui.painter().circle_filled(dot.center(), 3.5, AMBER());
                            let (name, email) = s.identity.as_ref().map_or((false, false), |i| {
                                (
                                    s.name != i.direct.name.as_deref().unwrap_or(""),
                                    s.email != i.direct.email.as_deref().unwrap_or(""),
                                )
                            });
                            let text = match (name, email) {
                                (true, true) => tr("Name and email changed"),
                                (true, false) => tr("Name changed"),
                                _ => tr("Email changed"),
                            };
                            ui.label(RichText::new(text).size(12.5).color(AMBER()));
                        }
                    });
                });
            }
        });
        if stop {
            s.cancel();
        }
        if discard {
            if let Some(i) = &s.identity {
                s.name = i.direct.name.clone().unwrap_or_default();
                s.email = i.direct.email.clone().unwrap_or_default();
            }
            if s.about_identity {
                s.notice = None;
            }
        }
        if retry {
            self.git_settings.error = None;
            self.git_settings.notice = None;
            self.load_git_settings()
        } else if let Some(choice) = choose.filter(|_| !busy) {
            self.choose_git(choice)
        } else if verify_managed && !busy {
            self.verify_bundled_git()
        } else if save_identity
            && let (Some(backend), Some(expected)) =
                (self.backend.clone(), self.git_settings.identity.clone())
        {
            let value = identity_value(&self.git_settings.name, &self.git_settings.email);
            let scope = self.browse_root.clone();
            self.git_settings.start(Job::Identity, move |cancel|{
                if let Some(scope)=scope {
                    let parent=expected.path.parent().ok_or_else(||Error::InvalidInput("Invalid global config path".into()))?.canonicalize().map_err(|e|io("resolve global config folder",e))?;
                    if !parent.starts_with(scope.canonicalize().map_err(|e|io("resolve test folder",e))?){return Err(Error::InvalidInput("Automation may only change isolated global config inside its test folder".into()))}
                }
                backend.save_global_identity(&expected,&value,&cancel)?;
                Ok(Finished::IdentitySaved(backend.global_identity(&CancellationToken::new()).map_err(|e|user_error(&e))))
            });
        }
        browse
    }
}
/// One way to run Git: a radio mark, a title over its detail (a path in monospace), and
/// `side` at the right. The whole row is the control; a row that cannot be chosen is dimmed.
fn choice_row(
    ui: &mut Ui,
    selected: bool,
    usable: bool,
    title: &str,
    detail: &str,
    mono: bool,
    side: impl FnOnce(&mut Ui),
) -> Response {
    let (rect, response) = ui.allocate_exact_size(
        vec2(ui.available_width(), 52.0),
        if usable { Sense::click() } else { Sense::hover() },
    );
    let enabled = ui.is_enabled() && usable;
    let hot = enabled && (response.hovered() || response.has_focus());
    if selected {
        ui.painter().rect_filled(rect, 0.0, BLUE().gamma_multiply(0.06));
        ui.painter()
            .rect_filled(Rect::from_min_size(rect.min, vec2(3.0, rect.height())), 0.0, BLUE());
    } else if hot {
        ui.painter().rect_filled(rect, 0.0, HOVER());
    }
    ui.painter()
        .hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0, CARD_LINE()));
    let fade = |color: Color32| {
        if enabled || selected {
            color
        } else {
            color.gamma_multiply(0.55)
        }
    };
    let mark = pos2(rect.left() + 22.0, rect.center().y);
    ui.painter().circle_stroke(
        mark,
        7.0,
        Stroke::new(1.5, fade(if selected { BLUE() } else { FAINT() })),
    );
    if selected {
        ui.painter().circle_filled(mark, 3.5, BLUE());
    }
    // The side first, so the title and path stop short of it.
    let mut side_ui = ui.new_child(
        UiBuilder::new()
            .max_rect(Rect::from_min_max(
                pos2(rect.center().x, rect.top()),
                pos2(rect.right() - 14.0, rect.bottom()),
            ))
            .layout(Layout::right_to_left(Align::Center)),
    );
    side_ui.spacing_mut().item_spacing.x = 8.0;
    side(&mut side_ui);
    let width = (rect.right() - 26.0 - side_ui.min_rect().width() - (rect.left() + 40.0)).max(40.0);
    let line = |text: &str, font: FontId, color: Color32| {
        let mut job = text::LayoutJob::simple_singleline(text.to_owned(), font, color);
        job.wrap = text::TextWrapping::truncate_at_width(width);
        ui.painter().layout_job(job)
    };
    let heading = line(
        title,
        if selected { semibold(13.0) } else { FontId::proportional(13.0) },
        fade(TEXT()),
    );
    let font = if mono { FontId::monospace(11.5) } else { FontId::proportional(12.0) };
    let path = line(detail, font, fade(HINT()));
    let top = rect.center().y - (heading.size().y + 3.0 + path.size().y) / 2.0;
    let below = top + heading.size().y + 3.0;
    ui.painter().galley(pos2(rect.left() + 40.0, top), heading, TEXT());
    ui.painter().galley(pos2(rect.left() + 40.0, below), path, HINT());
    response.widget_info(|| WidgetInfo::selected(Role::RadioButton, enabled, selected, title));
    response.on_hover_text(detail)
}
/// The version `git --version` gave, or why the copy cannot be used.
fn check_pill(ui: &mut Ui, check: Option<&Check>) {
    match check {
        Some(Check::Ready(version)) => {
            pages::pill(ui, version, MUTED());
        }
        Some(Check::Restricted(details)) => {
            pages::pill(ui, &trf!("{version} · HTTP blocked", version = details.0), AMBER()).on_hover_text(details.1);
        }
        Some(Check::Old(version)) => {
            pages::pill(ui, &trf!("{version} · needs 2.46", version), AMBER());
        }
        Some(Check::Broken(reason)) => {
            pages::pill(ui, tr("Not usable"), AMBER()).on_hover_text(reason);
        }
        None => {}
    }
}
fn identity_value(name: &str, email: &str) -> LocalIdentity {
    LocalIdentity {
        name: (!name.is_empty()).then(|| name.to_owned()),
        email: (!email.is_empty()).then(|| email.to_owned()),
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selection_save_is_atomic_checked_and_cancel_safe() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../work/git-preferences-0.80/selection-test")
            .join(format!(
                "{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
        fs::create_dir_all(&root).unwrap();
        let file = root.join(FILE);
        let token = CancellationToken::new();
        let git = GitClient::discover().unwrap();
        let choice = Choice::Path(git.executable().into());
        save_selection(&root, None, &choice, &token).unwrap();
        assert_eq!(selection(&root).unwrap().as_deref(), Some(git.executable()));
        let observed = read_file(&file).unwrap();
        fs::write(&file, "ROCKET-GIT-1\n").unwrap();
        assert!(
            save_selection(&root, observed.as_deref(), &choice, &token).is_err()
        );
        assert_eq!(selection(&root).unwrap(), None);
        let observed = read_file(&file).unwrap();
        let cancel = CancellationToken::new();
        cancel.cancel();
        assert!(matches!(
            save_selection(&root, observed.as_deref(), &choice, &cancel),
            Err(Error::Cancelled)
        ));
        assert_eq!(read_file(&file).unwrap(), observed);
        assert!(!root.join("git-selection.lock").exists());
        assert!(decode(Some(b"ROCKET-GIT-1\nrelative/git.exe")).is_err());
        assert!(decode(Some(b"unknown")).is_err());
        // A malformed file can be explicitly replaced after observing its exact bytes.
        fs::write(&file, "damaged").unwrap();
        save_selection(&root, Some(b"damaged"), &Choice::Automatic, &token).unwrap();
        assert_eq!(selection(&root).unwrap(), None);
    }
    /// Choosing a Git checks it and saves it at once. A program that fails the check
    /// leaves the saved choice in place and keeps the Custom path's text.
    #[test]
    fn a_git_choice_is_checked_and_saved_at_once() {
        crate::test_common::isolated(
            "ROCKET_GIT_SETTINGS_CASE",
            "git_settings::tests::a_git_choice_is_checked_and_saved_at_once",
            || {
                use crate::batch::flow_tests::{click, open, wait};
                let f = crate::test_common::Fixture::new();
                f.write("a.txt", "1\n");
                f.commit("base");
                let ctx = Context::default();
                let mut app = open(&ctx, &f);
                app.show_preferences(crate::preferences::Page::Git);
                // The page starts its read when it is first drawn.
                wait(&ctx, &mut app);
                wait(&ctx, &mut app);
                assert!(app.git_settings.loaded && app.git_settings.automatic);
                let missing = f.root.join("missing").join("git.exe").to_string_lossy().into_owned();
                app.git_settings.picked(missing.clone());
                wait(&ctx, &mut app);
                wait(&ctx, &mut app);
                let s = &app.git_settings;
                assert!(s.automatic, "a failed check keeps the saved choice");
                assert!(s.error.as_deref().is_some_and(|e| e.starts_with(&missing)), "{:?}", s.error);
                assert_eq!(s.custom, missing);
                assert_eq!(selection(&app.state_directory).unwrap(), None);
                // The Git this window runs, as the Custom path: checked and saved.
                let git = app.backend.as_ref().unwrap().executable.to_string_lossy().into_owned();
                app.git_settings.custom = git.clone();
                click(&ctx, &mut app, "git-save");
                wait(&ctx, &mut app);
                let s = &app.git_settings;
                assert!(!s.automatic && s.error.is_none(), "{:?}", s.error);
                assert_eq!(selection(&app.state_directory).unwrap().as_deref(), Some(Path::new(&git)));
                // Automatic again, from its row.
                click(&ctx, &mut app, "git-automatic");
                wait(&ctx, &mut app);
                assert!(app.git_settings.automatic && app.git_settings.error.is_none());
                assert_eq!(selection(&app.state_directory).unwrap(), None);
            },
        );
    }
    #[test]
    fn versions_preserve_security_builds() {
        assert_eq!(display_version("git version 2.56.0.windows.3"), "2.56.0.windows.3");
        assert_eq!(display_version("git version 2.56.0.windows.4"), "2.56.0.windows.4");
        assert_eq!(display_version("git version 2.45.2"), "2.45.2");
        assert!(Found {path: PathBuf::from("unused"), check: Some(Check::Restricted(Box::new(("2.56.0.windows.3".into(), "HTTP blocked"))))}.usable());
    }
    #[test]
    fn managed_choice_is_relocatable_and_keeps_legacy_selections() {
        let data = Choice::Managed.encode().unwrap();
        assert_eq!(data, b"ROCKET-GIT-2\nmanaged");
        assert_eq!(decode(Some(&data)).unwrap(), Choice::Managed);
        assert_eq!(decode(Some(b"ROCKET-GIT-1\n")).unwrap(), Choice::Automatic);
        let old = Choice::Path(PathBuf::from("D:/Git Unicode/git.exe"));
        assert_eq!(decode(Some(&old.encode().unwrap())).unwrap(), old);
        assert!(decode(Some(b"ROCKET-GIT-2\nmanaged\nanything")).is_err());
        let mut state = State { saved: Choice::Managed, ..Default::default() };
        state.show_saved();
        assert!(state.managed && !state.automatic);
        assert_eq!(state.choice_for("git-managed").as_deref(), Some(MANAGED));
        assert_eq!(state.choice(), MANAGED);
    }
}
