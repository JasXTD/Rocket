//! Preferences → Terminal: the shell the Terminal drawer runs. A custom program and its
//! arguments come only from this profile's settings, like tool profiles: repository
//! configuration never supplies them. An MCP test window can't change the setting and keeps
//! Rocket's own shell (Windows PowerShell, or bash on Linux), whose probes and private
//! history it relies on. Each platform lists its own shells.
use crate::{
    i18n::{tr, trf},
    app::{Rocket, UiControl},
    pages,
    preferences::{card_frame, card_header, status_line},
    repository_forms::{Form, Status},
    style::*,
};
use eframe::egui::*;
use rocket_terminal::Shell;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

fn choice_label(choice: Choice) -> &'static str {
    match choice {
        Choice::CommandPrompt => tr("Command Prompt"),
        Choice::LoginShell => tr("Login shell"),
        Choice::Custom => tr("Custom"),
        _ => choice.label(),
    }
}

/// The label column of the card's rows.
const LABEL: f32 = 96.0;
/// A custom shell gets as many arguments as a tool profile.
const MAX_ARGUMENTS: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice {
    WindowsPowerShell,
    GitBash,
    CommandPrompt,
    PowerShell7,
    /// Linux: the shell $SHELL names, the user's login shell.
    LoginShell,
    Bash,
    Zsh,
    Fish,
    Custom,
}
impl Default for Choice {
    fn default() -> Self {
        Self::ALL[0]
    }
}
impl Choice {
    /// This platform's shells, the default first.
    #[cfg(windows)]
    pub const ALL: [Self; 5] = [
        Self::WindowsPowerShell,
        Self::GitBash,
        Self::CommandPrompt,
        Self::PowerShell7,
        Self::Custom,
    ];
    #[cfg(not(any(windows, target_os = "macos")))]
    pub const ALL: [Self; 5] = [Self::LoginShell, Self::Bash, Self::Zsh, Self::Fish, Self::Custom];
    // Finder need not provide $SHELL. macOS always has /bin/zsh; selecting an
    // explicit login/custom shell remains available without altering profiles.
    #[cfg(target_os = "macos")]
    pub const ALL: [Self; 5] = [Self::Zsh, Self::LoginShell, Self::Bash, Self::Fish, Self::Custom];
    pub fn label(self) -> &'static str {
        match self {
            Self::WindowsPowerShell => "Windows PowerShell",
            Self::GitBash => "Git Bash",
            Self::CommandPrompt => "Command Prompt",
            Self::PowerShell7 => "PowerShell 7",
            Self::LoginShell => "Login shell",
            Self::Bash => "Bash",
            Self::Zsh => "Zsh",
            Self::Fish => "Fish",
            Self::Custom => "Custom",
        }
    }
    /// Its automation name.
    pub fn key(self) -> &'static str {
        match self {
            Self::WindowsPowerShell => "windows-powershell",
            Self::GitBash => "git-bash",
            Self::CommandPrompt => "command-prompt",
            Self::PowerShell7 => "powershell-7",
            Self::LoginShell => "login-shell",
            Self::Bash => "bash",
            Self::Zsh => "zsh",
            Self::Fish => "fish",
            Self::Custom => "custom",
        }
    }
}

/// What an MCP window runs, whatever is saved: Rocket's own shell.
pub(crate) const ROCKET_SHELL: &str = if cfg!(windows) { "Windows PowerShell" } else { "bash" };

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Settings {
    pub choice: Choice,
    /// Custom's program and arguments, one per line. They stay while another shell is
    /// chosen, so going back to Custom finds them.
    pub program: String,
    pub arguments: String,
}
impl Settings {
    pub(crate) fn valid(&self) -> bool {
        self.program.len() <= 32768
            && self.arguments.len() <= 16384
            && !self.program.contains(['\0', '\r', '\n'])
            && !self.arguments.contains('\0')
    }
    /// What the drawer calls this shell. The login shell is named after its program.
    pub(crate) fn name(&self) -> String {
        let named = |program: &Path| {
            program
                .file_stem()
                .map(|name| name.to_string_lossy().into_owned())
                .filter(|name| !name.is_empty())
        };
        match self.choice {
            Choice::Custom => named(Path::new(self.program.trim())).unwrap_or_else(|| "Custom shell".to_owned()),
            Choice::LoginShell => login_shell().as_deref().and_then(named).unwrap_or_else(|| self.choice.label().to_owned()),
            choice => choice.label().to_owned(),
        }
    }
    fn argument_lines(&self) -> Vec<String> {
        self.arguments.lines().filter(|l| !l.is_empty()).map(str::to_owned).collect()
    }
    /// Why this custom program can't be used, before looking for the file.
    fn custom_problem(&self) -> Option<&'static str> {
        let program = self.program.trim();
        let path = Path::new(program);
        // Windows programs are .exe files; on Linux any file can be one.
        let program_file = !cfg!(windows) || path.extension().is_some_and(|x| x.eq_ignore_ascii_case("exe"));
        if program.is_empty() {
            Some(crate::i18n::mark("Choose the shell's program."))
        } else if !path.is_absolute() || !program_file {
            Some(if cfg!(windows) { crate::i18n::mark("Use the full path to a program's .exe file.") } else { crate::i18n::mark("Use the full path to the shell's program.") })
        } else if self.argument_lines().len() > MAX_ARGUMENTS {
            Some(crate::i18n::mark("Use at most 64 arguments, one per line."))
        } else {
            None
        }
    }
}

/// The shells that may not be here, looked for when the page opens or a terminal starts.
/// Windows fills the first three, Linux the rest.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Located {
    git_bash: Option<PathBuf>,
    powershell_7: Option<PathBuf>,
    git_version: String,
    login: Option<PathBuf>,
    bash: Option<PathBuf>,
    zsh: Option<PathBuf>,
    fish: Option<PathBuf>,
}
impl Located {
    fn installed(&self, choice: Choice) -> Option<&PathBuf> {
        match choice {
            Choice::LoginShell => self.login.as_ref(),
            Choice::Bash => self.bash.as_ref(),
            Choice::Zsh => self.zsh.as_ref(),
            Choice::Fish => self.fish.as_ref(),
            _ => None,
        }
    }
}

/// The program $SHELL names, when it is a full path. Nothing is looked up on PATH.
fn login_shell() -> Option<PathBuf> {
    std::env::var_os("SHELL").map(PathBuf::from).filter(|path| path.is_absolute())
}

/// Where `name` is installed: the first entry of /etc/shells, the system's list of
/// installed shells, with that file name, else /usr/bin/`name`. Only regular files count.
fn installed_shell(name: &str, shells: Option<&str>) -> Option<PathBuf> {
    shells
        .into_iter()
        .flat_map(str::lines)
        .map(str::trim)
        .filter(|line| line.starts_with('/'))
        .map(PathBuf::from)
        .filter(|path| path.file_name().is_some_and(|file| file == name))
        .chain(Some(Path::new("/usr/bin").join(name)))
        .chain(cfg!(target_os = "macos").then(|| Path::new("/bin").join(name)))
        .find(|path| path.is_file())
}

/// `cmd\git.exe`, `bin\git.exe` or `ucrt64\bin\git.exe`: the launcher is in the install's
/// own `bin`. Rocket's bundled Git has one, so nothing needs installing.
fn beside_git(git: &Path) -> Option<PathBuf> {
    git.ancestors().skip(1).take(3).map(|root| root.join(r"bin\bash.exe")).find(|path| path.is_file())
}
fn powershell_7() -> Option<PathBuf> {
    let path = PathBuf::from(std::env::var_os("ProgramFiles")?).join(r"PowerShell\7\pwsh.exe");
    path.is_file().then_some(path)
}

/// Whether the shell can be used: `Ready`, or the pill and the line that say why not.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Readiness {
    Ready(String),
    /// A custom program's file is still being looked for.
    Checking,
    NotAvailable(String),
    NotFound(String),
    CantUse(String),
}

fn readiness(settings: &Settings, located: &Located, custom: Option<&Status>) -> Readiness {
    if !Choice::ALL.contains(&settings.choice) {
        return Readiness::CantUse(trf!("{shell} isn't on this system. Choose another shell.", shell = choice_label(settings.choice)));
    }
    match settings.choice {
        Choice::LoginShell => match (located.login.as_ref(), login_shell()) {
            (Some(_), _) => Readiness::Ready(tr("Your login shell, from $SHELL").into()),
            (None, Some(path)) => Readiness::NotFound(trf!("$SHELL names {path}, which isn't there. Choose another shell.", path = path.display())),
            (None, None) => Readiness::NotFound(tr("$SHELL doesn't name a shell. Choose another shell.").into()),
        },
        Choice::Bash | Choice::Zsh | Choice::Fish => match located.installed(settings.choice) {
            Some(_) => Readiness::Ready(tr("Installed").into()),
            None => Readiness::NotFound(trf!("{shell} isn't installed. Choose another shell.", shell = choice_label(settings.choice))),
        },
        Choice::WindowsPowerShell | Choice::CommandPrompt => Readiness::Ready(tr("Comes with Windows").into()),
        Choice::GitBash if located.git_bash.is_some() => {
            // "git version 2.56.0.windows.4": its prefix and build suffix add nothing here.
            let version = located.git_version.trim_start_matches("git version ");
            let version = version.split(".windows").next().unwrap_or_default();
            Readiness::Ready(trf!("From the Git Rocket uses · Git {version}", version))
        }
        Choice::GitBash => Readiness::NotAvailable(tr("Pick a full Git for Windows, or choose Custom.").into()),
        Choice::PowerShell7 if located.powershell_7.is_some() => Readiness::Ready(tr("Found").into()),
        Choice::PowerShell7 => Readiness::NotFound(tr("PowerShell 7 isn't installed any more. Choose another shell.").into()),
        Choice::Custom => {
            if let Some(problem) = settings.custom_problem() {
                return Readiness::CantUse(tr(problem).into());
            }
            match custom {
                Some(Status::Exists) => Readiness::Ready(tr("Found. It runs at New Terminal, not before.").into()),
                Some(Status::Refused(reason)) => Readiness::NotFound(reason.clone()),
                Some(_) => Readiness::NotFound(tr("File not found. Save stays off until it exists.").into()),
                None => Readiness::Checking,
            }
        }
    }
}

/// What New Terminal starts, or why it can't.
fn launch(settings: &Settings, located: &Located) -> Result<Shell, String> {
    if !Choice::ALL.contains(&settings.choice) {
        return Err(format!("{} isn't on this system.", settings.choice.label()));
    }
    match settings.choice {
        // Linux shells start with no arguments and run interactively on the terminal.
        Choice::LoginShell | Choice::Bash | Choice::Zsh | Choice::Fish => located
            .installed(settings.choice)
            .map(|program| Shell::Custom { program: program.clone(), arguments: vec![] })
            .ok_or_else(|| match settings.choice {
                Choice::LoginShell => "$SHELL doesn't name an installed shell.".to_owned(),
                choice => format!("{} isn't installed.", choice.label()),
            }),
        Choice::WindowsPowerShell => Ok(Shell::WindowsPowerShell),
        Choice::CommandPrompt => Ok(Shell::CommandPrompt),
        Choice::GitBash => located
            .git_bash
            .clone()
            .map(Shell::GitBash)
            .ok_or_else(|| "The Git Rocket uses has no bash.exe.".to_owned()),
        Choice::PowerShell7 => located
            .powershell_7
            .clone()
            .map(Shell::PowerShell)
            .ok_or_else(|| "PowerShell 7 isn't installed.".to_owned()),
        Choice::Custom => match settings.custom_problem() {
            Some(problem) => Err(problem.to_owned()),
            None => Ok(Shell::Custom {
                program: PathBuf::from(settings.program.trim()),
                arguments: settings.argument_lines(),
            }),
        },
    }
}

/// What the Program and Arguments rows show for a built-in shell.
fn shown(settings: &Settings, located: &Located) -> (String, String) {
    let shell = launch(settings, located).ok();
    let program = shell
        .as_ref()
        .and_then(|s| s.program().ok())
        .map_or_else(String::new, |p| crate::views::display_path(&p));
    let arguments = shell
        .as_ref()
        .and_then(Shell::arguments)
        .map_or_else(|| tr("Set by Rocket").to_owned(), |a| if a.is_empty() { tr("None").to_owned() } else { a.join(" ") });
    (program, arguments)
}

/// The Program row when the chosen shell isn't there.
fn missing_program(choice: Choice) -> &'static str {
    match choice {
        Choice::GitBash => tr("No bash.exe beside this Git"),
        Choice::LoginShell => tr("$SHELL doesn't name an installed shell"),
        _ => tr("Not installed"),
    }
}

pub struct State {
    pub saved: Settings,
    pub editing: Settings,
    /// Off in an MCP window: only the user changes the shell.
    pub editable: bool,
    /// Local acceptance runner selects only /bin/zsh with startup files disabled.
    #[cfg(target_os = "macos")]
    pub fixture_zsh: bool,
    located: Located,
    check: Form,
    /// Whether this frame's edit can be saved; the footer draws after the page.
    pub(crate) savable: bool,
}
impl Default for State {
    fn default() -> Self {
        Self {
            saved: Default::default(),
            editing: Default::default(),
            editable: true,
            #[cfg(target_os = "macos")]
            fixture_zsh: false,
            located: Default::default(),
            check: Default::default(),
            savable: false,
        }
    }
}
impl State {
    pub(crate) fn load(&mut self, saved: Settings) {
        self.editing = saved.clone();
        self.saved = saved;
    }
    pub fn dirty(&self) -> bool {
        self.editing != self.saved
    }
    pub(crate) fn save(&mut self) {
        if self.savable {
            self.saved = self.editing.clone();
        }
    }
    pub(crate) fn discard(&mut self) {
        self.editing = self.saved.clone();
    }
    pub(crate) fn browse_start(&self) -> String {
        self.editing.program.clone()
    }
    pub(crate) fn program_picked(&mut self, program: String) {
        self.editing.program = program;
    }
    fn choose(&mut self, choice: Choice) {
        // Custom starts from the shell you had, so a small change stays small.
        if choice == Choice::Custom && self.editing.program.trim().is_empty() {
            let (program, _) = shown(&self.editing, &self.located);
            self.editing.arguments = launch(&self.editing, &self.located)
                .ok()
                .and_then(|shell| shell.arguments())
                .unwrap_or_default()
                .join("\n");
            self.editing.program = program;
        }
        self.editing.choice = choice;
    }
}

pub(crate) fn check_program(path: &Path) -> Status {
    match path.symlink_metadata() {
        Ok(found) if found.is_dir() => Status::Refused(if cfg!(windows) {
            "This is a folder. Use the full path to a program's .exe file."
        } else {
            "This is a folder. Use the full path to a program."
        }.into()),
        #[cfg(unix)]
        Ok(_) if !executable(path) => Status::Refused("This file can't be run: it isn't executable.".into()),
        Ok(_) => Status::Exists,
        Err(_) => Status::MissingParent,
    }
}

/// Whether the file a link or path names has an executable bit.
#[cfg(unix)]
fn executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata().is_ok_and(|found| found.is_file() && found.permissions().mode() & 0o111 != 0)
}

/// A row with its label in the card's column, level with the top of the field.
fn row(ui: &mut Ui, label: &str, content: impl FnOnce(&mut Ui)) {
    Frame::new().inner_margin(Margin::symmetric(14, 4)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            let (rect, _) = ui.allocate_exact_size(vec2(LABEL, CONTROL), Sense::hover());
            ui.painter().text(rect.left_center(), Align2::LEFT_CENTER, label, FontId::proportional(13.0), MUTED());
            ui.vertical(|ui| {
                ui.set_width(ui.available_width());
                content(ui);
            });
        });
    });
}

/// A value that can't be edited, shaped like a field.
fn fixed(ui: &mut Ui, widgets: &mut HashMap<String, UiControl>, key: &str, value: &str, mono: bool) {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), CONTROL), Sense::hover());
    ui.painter().rect(rect, 6.0, PANEL(), Stroke::new(1.0, CARD_STROKE()), StrokeKind::Inside);
    let font = if mono { FontId::monospace(12.0) } else { FontId::proportional(13.0) };
    let mut job = text::LayoutJob::simple_singleline(value.into(), font, MUTED());
    job.wrap = text::TextWrapping::truncate_at_width((rect.width() - 18.0).max(0.0));
    let galley = ui.painter().layout_job(job);
    ui.painter().galley(pos2(rect.left() + 9.0, rect.center().y - galley.size().y / 2.0), galley, MUTED());
    if mono {
        response.clone().on_hover_text(value);
    }
    pages::control(widgets, key, &response, ui.clip_rect());
}

/// A shell in the dropdown, with where it comes from at the right.
fn choice_row(ui: &mut Ui, label: &str, detail: &str, selected: bool, enabled: bool) -> Response {
    let response = ui.add_enabled(enabled, Button::new("").frame(false).min_size(vec2(ui.available_width(), CONTROL)));
    let rect = response.rect;
    let painter = ui.painter();
    if enabled && (response.hovered() || response.has_focus()) {
        painter.rect_filled(rect, 6.0, HOVER());
    }
    if selected {
        let mark = Rect::from_center_size(pos2(rect.left() + 15.0, rect.center().y), vec2(12.0, 12.0));
        icon(painter, mark, Icon::Check, BLUE());
    }
    let ink = if enabled { TEXT() } else { FAINT() };
    painter.text(pos2(rect.left() + 30.0, rect.center().y), Align2::LEFT_CENTER, label, FontId::proportional(13.0), ink);
    painter.text(pos2(rect.right() - 9.0, rect.center().y), Align2::RIGHT_CENTER, detail, FontId::proportional(12.0), HINT());
    response.widget_info(|| WidgetInfo::selected(WidgetType::Button, enabled, selected, label));
    response
}

impl Rocket {
    /// Looks for Git Bash and PowerShell 7, or on Linux the login shell, Bash, Zsh and
    /// Fish. A few checks for a file, like the ones behind Preferences → Commit messages,
    /// and only on request, never per frame.
    fn locate_shells(&self) -> Located {
        if cfg!(windows) {
            let git = self.backend.as_ref();
            return Located {
                git_bash: git.and_then(|b| beside_git(&b.executable)),
                powershell_7: powershell_7(),
                git_version: git.map_or_else(String::new, |b| b.version.clone()),
                ..Default::default()
            };
        }
        let shells = std::fs::read_to_string("/etc/shells").ok();
        let shell = |name| installed_shell(name, shells.as_deref());
        Located {
            login: login_shell().filter(|path| path.is_file()),
            bash: shell("bash"),
            zsh: shell("zsh"),
            fish: shell("fish"),
            ..Default::default()
        }
    }
    /// The page opened: it shows what is on this computer now.
    pub(crate) fn start_terminal_page(&mut self) {
        self.terminal_shell.located = self.locate_shells();
        self.terminal_shell.check = Default::default();
    }
    /// What New Terminal names, before and after a session starts. An MCP window always
    /// runs Rocket's own shell, whatever is saved.
    pub(crate) fn terminal_name(&self) -> String {
        #[cfg(target_os = "macos")]
        if self.terminal_history_file.is_some() && self.terminal_shell.fixture_zsh { return "zsh (isolated)".into(); }
        if self.terminal_history_file.is_some() {
            ROCKET_SHELL.to_owned()
        } else {
            self.terminal_shell.saved.name()
        }
    }
    /// What New Terminal starts, or why it can't.
    pub(crate) fn terminal_launch(&self) -> Result<Shell, String> {
        if self.terminal_history_file.is_some() {
            #[cfg(target_os = "macos")]
            if self.terminal_shell.fixture_zsh {
                return Ok(Shell::Custom { program: "/bin/zsh".into(), arguments: vec!["-dfi".into()] });
            }
            return Ok(Shell::WindowsPowerShell);
        }
        launch(&self.terminal_shell.saved, &self.locate_shells())
    }
    /// The Shell card. Returns whether Browse was clicked, and Git settings asks for that
    /// page through `open_git`.
    pub(crate) fn terminal_page(&mut self, ui: &mut Ui, open_git: &mut bool) -> bool {
        let widgets = &mut self.frame_widgets;
        let s = &mut self.terminal_shell;
        let editable = s.editable;
        let dialog_layer = ui.layer_id();
        // What's checked is the file a custom shell names; the rest was located on opening.
        let wanted = (s.editing.choice == Choice::Custom && s.editing.custom_problem().is_none())
            .then(|| PathBuf::from(s.editing.program.trim()));
        let checked = s.check.status(ui.ctx(), wanted, None, check_program);
        let ready = readiness(&s.editing, &s.located, checked.as_ref());
        let open_terminal = self.tabs.iter()
            .filter_map(|tab| tab.terminal.as_ref())
            .find(|t| !t.status.closed && t.name != s.editing.name())
            .map(|t| t.name.clone());
        s.savable = editable && matches!(ready, Readiness::Ready(_));
        let (color, text) = match &ready {
            Readiness::Ready(text) => (GREEN(), text.as_str()),
            Readiness::Checking => (MUTED(), tr("Looking for the file")),
            Readiness::NotAvailable(text) | Readiness::NotFound(text) => (AMBER(), text.as_str()),
            Readiness::CantUse(text) => (RED(), text.as_str()),
        };
        let mut browse = false;
        let mut picked = None;
        card_frame(ui, |ui| {
            card_header(ui, tr("Shell"), |ui| {
                if editable {
                    let label = match &ready {
                        Readiness::Ready(_) => tr("Ready"),
                        Readiness::Checking => tr("Checking"),
                        Readiness::NotAvailable(_) => tr("Not available"),
                        Readiness::NotFound(_) => tr("Not found"),
                        Readiness::CantUse(_) => tr("Can't use"),
                    };
                    pages::pill(ui, label, color);
                } else {
                    ui.label(RichText::new(tr("Locked")).size(12.0).color(HINT()));
                    let (rect, _) = ui.allocate_exact_size(vec2(13.0, 13.0), Sense::hover());
                    icon(ui.painter(), rect, Icon::Lock, FAINT());
                }
            });
            ui.add_space(6.0);
            ui.add_enabled_ui(editable, |ui| {
                row(ui, tr("Shell"), |ui| {
                    let combo = ComboBox::from_id_salt("shell-choice")
                        .selected_text(choice_label(s.editing.choice))
                        .width(ui.available_width())
                        .show_ui(ui, |ui| {
                            // Modal and popup share a layer order; keep the list above the dialog.
                            ui.ctx().set_sublayer(dialog_layer, ui.layer_id());
                            ui.spacing_mut().item_spacing.y = 0.0;
                            for choice in Choice::ALL {
                                let (detail, enabled) = match choice {
                                    Choice::WindowsPowerShell => (tr("Default"), true),
                                    Choice::GitBash if s.located.git_bash.is_some() => (tr("From the Git Rocket uses"), true),
                                    Choice::GitBash => (tr("Not in this Git"), false),
                                    Choice::CommandPrompt => (tr("Comes with Windows"), true),
                                    Choice::PowerShell7 if s.located.powershell_7.is_some() => (tr("Installed"), true),
                                    Choice::PowerShell7 => (tr("Not installed"), false),
                                    Choice::LoginShell if s.located.login.is_some() => (tr("Default · from $SHELL"), true),
                                    Choice::LoginShell => (tr("$SHELL isn't set"), false),
                                    Choice::Bash | Choice::Zsh | Choice::Fish if s.located.installed(choice).is_some() => (tr("Installed"), true),
                                    Choice::Bash | Choice::Zsh | Choice::Fish => (tr("Not installed"), false),
                                    Choice::Custom => (tr("Your own program"), true),
                                };
                                if choice == Choice::Custom {
                                    ui.add_space(4.0);
                                    ui.separator();
                                    ui.add_space(4.0);
                                }
                                let r = choice_row(ui, choice_label(choice), detail, choice == s.editing.choice, enabled);
                                pages::control(widgets, format!("shell-choice-{}", choice.key()), &r, ui.clip_rect());
                                if r.clicked() {
                                    picked = Some(choice);
                                    ui.close();
                                }
                            }
                        });
                    pages::control(widgets, "shell-choice", &combo.response, ui.clip_rect());
                });
                if s.editing.choice == Choice::Custom {
                    row(ui, tr("Program"), |ui| {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 8.0;
                            // As wide as its label needs: Browse is a longer word in
                            // some languages.
                            let button = pages::button_width(ui, tr("Browse"), 86.0);
                            let width = (ui.available_width() - button - 8.0).max(80.0);
                            let r = ui.add_sized(
                                [width, CONTROL],
                                TextEdit::singleline(&mut s.editing.program)
                                    .hint_text(RichText::new(if cfg!(windows) { tr("Full path to the shell's .exe") } else { tr("Full path to the shell's program") }).font(FontId::proportional(13.0)).color(FAINT()))
                                    .font(FontId::monospace(12.0))
                                    .vertical_align(Align::Center)
                                    .char_limit(32768)
                                    .margin(Margin::symmetric(8, 2)),
                            );
                            pages::control(widgets, "shell-program", &r, ui.clip_rect());
                            if !matches!(ready, Readiness::Ready(_) | Readiness::Checking) {
                                ui.painter().rect_stroke(r.rect, 6.0, Stroke::new(1.0, color.gamma_multiply(0.6)), StrokeKind::Inside);
                            }
                            let r = ui.add_sized([button, CONTROL], Button::new(tr("Browse")));
                            pages::control(widgets, "shell-browse", &r, ui.clip_rect());
                            browse = r.clicked();
                        });
                    });
                    row(ui, tr("Arguments"), |ui| {
                        crate::external_tools::arguments_editor(ui, widgets, "shell-arguments", &mut s.editing.arguments, true);
                    });
                } else {
                    let (program, arguments) = shown(&s.editing, &s.located);
                    row(ui, tr("Program"), |ui| {
                        if program.is_empty() {
                            fixed(ui, widgets, "shell-program", missing_program(s.editing.choice), false);
                        } else {
                            fixed(ui, widgets, "shell-program", &program, true);
                        }
                    });
                    let set_by_rocket = launch(&s.editing, &s.located).is_ok_and(|shell| shell.arguments().is_none());
                    row(ui, tr("Arguments"), |ui| fixed(ui, widgets, "shell-arguments", &arguments, !set_by_rocket));
                }
            });
            // Under the fields, not the labels.
            Frame::new()
                .inner_margin(Margin { left: 14 + LABEL as i8 + 12, right: 14, top: 4, bottom: 12 })
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.spacing_mut().item_spacing.y = 6.0;
                    if editable {
                        ui.horizontal(|ui| {
                            status_line(ui, color, text);
                            if matches!(ready, Readiness::NotAvailable(_)) {
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    let r = ui.add(Button::new(RichText::new(tr("Git settings")).size(12.0)));
                                    pages::control(widgets, "shell-git-settings", &r, ui.clip_rect());
                                    *open_git = r.clicked();
                                });
                            }
                        });
                    } else {
                        status_line(ui, AMBER(), &trf!("MCP windows always use {shell}.", shell = ROCKET_SHELL));
                    }
                    if let Some(name) = &open_terminal {
                        status_line(ui, AMBER(), &trf!("Open terminals keep {name} until you end them.", name));
                    }
                });
        });
        if let Some(choice) = picked {
            s.choose(choice);
        }
        browse
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_os = "macos")]
    #[test]
    fn macos_default_and_system_shells_do_not_require_finder_shell_environment() {
        assert_eq!(Settings::default().choice, Choice::Zsh);
        assert_eq!(installed_shell("zsh", None), Some(PathBuf::from("/bin/zsh")));
        assert_eq!(installed_shell("bash", None), Some(PathBuf::from("/bin/bash")));
        assert!(!Choice::ALL.contains(&Choice::WindowsPowerShell));
    }

    #[cfg(windows)]
    fn located() -> Located {
        Located {
            git_bash: Some(r"C:\Apps\Rocket\git\2.56.0.windows.4\bin\bash.exe".into()),
            powershell_7: None,
            git_version: "git version 2.56.0.windows.4".into(),
            ..Default::default()
        }
    }
    #[cfg(windows)]
    #[test]
    fn the_default_is_windows_powershell_and_custom_is_named_after_its_program() {
        assert_eq!(Settings::default().name(), "Windows PowerShell");
        let mut custom = Settings { choice: Choice::Custom, ..Default::default() };
        assert_eq!(custom.name(), "Custom shell");
        custom.program = r"C:\msys64\usr\bin\zsh.exe".into();
        assert_eq!(custom.name(), "zsh");
    }
    #[cfg(windows)]
    #[test]
    fn each_shell_is_ready_or_says_why_not() {
        let plain = |choice| Settings { choice, ..Default::default() };
        let ready = |s: &Settings| matches!(readiness(s, &located(), None), Readiness::Ready(_));
        assert!(ready(&plain(Choice::WindowsPowerShell)));
        assert!(ready(&plain(Choice::CommandPrompt)));
        assert_eq!(
            readiness(&plain(Choice::GitBash), &located(), None),
            Readiness::Ready("From the Git Rocket uses · Git 2.56.0".into())
        );
        let nothing = Located::default();
        assert!(matches!(readiness(&plain(Choice::GitBash), &nothing, None), Readiness::NotAvailable(_)));
        assert!(matches!(readiness(&plain(Choice::PowerShell7), &located(), None), Readiness::NotFound(_)));
        assert!(launch(&plain(Choice::GitBash), &nothing).is_err());
        assert_eq!(launch(&plain(Choice::WindowsPowerShell), &nothing), Ok(Shell::WindowsPowerShell));
    }
    #[cfg(windows)]
    #[test]
    fn a_custom_shell_must_be_a_full_exe_path_that_exists() {
        let custom = |program: &str| Settings { choice: Choice::Custom, program: program.into(), arguments: "--login\n\n-i".into() };
        let state = |s: &Settings, check| readiness(s, &located(), check);
        assert!(matches!(state(&custom(""), None), Readiness::CantUse(_)));
        assert!(matches!(state(&custom("zsh.exe"), None), Readiness::CantUse(_)));
        assert!(matches!(state(&custom(r"C:\Tools\shell.cmd"), None), Readiness::CantUse(_)));
        let good = custom(r"C:\msys64\usr\bin\zsh.exe");
        assert_eq!(state(&good, None), Readiness::Checking, "not looked for yet");
        assert!(matches!(state(&good, Some(&Status::MissingParent)), Readiness::NotFound(_)));
        assert!(matches!(state(&good, Some(&Status::Exists)), Readiness::Ready(_)));
        assert_eq!(
            launch(&good, &located()),
            Ok(Shell::Custom { program: r"C:\msys64\usr\bin\zsh.exe".into(), arguments: vec!["--login".into(), "-i".into()] })
        );
        assert!(launch(&custom("relative.exe"), &located()).is_err());
        let many = Settings { arguments: "x\n".repeat(65), ..good };
        assert!(matches!(state(&many, Some(&Status::Exists)), Readiness::CantUse(_)));
    }
    #[cfg(windows)]
    #[test]
    fn custom_starts_from_the_shell_you_had_and_keeps_its_own_values() {
        let mut state = State { located: located(), ..Default::default() };
        state.editing.choice = Choice::GitBash;
        state.choose(Choice::Custom);
        assert_eq!(state.editing.choice, Choice::Custom);
        assert!(state.editing.program.ends_with(r"\bin\bash.exe"));
        assert_eq!(state.editing.arguments, "--login\n-i");
        state.editing.program = r"D:\Tools\nu.exe".into();
        state.choose(Choice::CommandPrompt);
        state.choose(Choice::Custom);
        assert_eq!(state.editing.program, r"D:\Tools\nu.exe");
    }
    #[test]
    fn nothing_is_saved_until_save_and_only_when_the_shell_is_ready() {
        let mut state = State::default();
        state.editing.choice = Choice::CommandPrompt;
        assert!(state.dirty());
        state.save();
        assert_eq!(state.saved, Settings::default(), "not checked, not saved");
        state.savable = true;
        state.save();
        assert_eq!(state.saved.choice, Choice::CommandPrompt);
        state.editing.choice = Choice::GitBash;
        state.discard();
        assert!(!state.dirty());
    }
    #[cfg(windows)]
    #[test]
    fn the_launcher_is_found_in_the_gits_own_bin_folder() {
        let root = std::env::temp_dir().join(format!("rocket-beside-git-{}", std::process::id()));
        std::fs::create_dir_all(root.join("bin")).unwrap();
        std::fs::create_dir_all(root.join(r"ucrt64\bin")).unwrap();
        std::fs::create_dir_all(root.join("cmd")).unwrap();
        assert_eq!(beside_git(&root.join(r"cmd\git.exe")), None, "no bash here");
        std::fs::write(root.join(r"bin\bash.exe"), b"never run").unwrap();
        for git in [r"cmd\git.exe", r"bin\git.exe", r"ucrt64\bin\git.exe"] {
            assert_eq!(beside_git(&root.join(git)), Some(root.join(r"bin\bash.exe")), "{git}");
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(not(windows))]
    fn linux() -> Located {
        Located { login: Some("/bin/bash".into()), bash: Some("/usr/bin/bash".into()), ..Default::default() }
    }
    #[cfg(target_os = "linux")]
    #[test]
    fn linux_lists_its_own_shells_and_the_login_shell_comes_first() {
        assert_eq!(Choice::ALL, [Choice::LoginShell, Choice::Bash, Choice::Zsh, Choice::Fish, Choice::Custom]);
        assert_eq!(Settings::default().choice, Choice::LoginShell);
        let plain = |choice| Settings { choice, ..Default::default() };
        // Named after the program $SHELL names, so the drawer says "bash".
        let named = login_shell().and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()));
        assert_eq!(plain(Choice::LoginShell).name(), named.unwrap_or_else(|| "Login shell".into()));
        assert_eq!(plain(Choice::Zsh).name(), "Zsh");
        assert_eq!(ROCKET_SHELL, "bash");
    }
    #[cfg(not(windows))]
    #[test]
    fn linux_shells_are_ready_when_installed_and_start_with_no_arguments() {
        let plain = |choice| Settings { choice, ..Default::default() };
        let state = |choice, located: &Located| readiness(&plain(choice), located, None);
        assert_eq!(state(Choice::LoginShell, &linux()), Readiness::Ready("Your login shell, from $SHELL".into()));
        assert_eq!(state(Choice::Bash, &linux()), Readiness::Ready("Installed".into()));
        assert_eq!(state(Choice::Zsh, &linux()), Readiness::NotFound("Zsh isn't installed. Choose another shell.".into()));
        assert!(matches!(state(Choice::LoginShell, &Located::default()), Readiness::NotFound(_)));
        assert_eq!(launch(&plain(Choice::Bash), &linux()), Ok(Shell::Custom { program: "/usr/bin/bash".into(), arguments: vec![] }));
        assert_eq!(launch(&plain(Choice::LoginShell), &linux()), Ok(Shell::Custom { program: "/bin/bash".into(), arguments: vec![] }));
        assert_eq!(launch(&plain(Choice::Fish), &linux()), Err("Fish isn't installed.".into()));
        assert_eq!(shown(&plain(Choice::Bash), &linux()), ("/usr/bin/bash".into(), "None".into()));
        assert_eq!(missing_program(Choice::Fish), "Not installed");
        // A setting saved for Windows' shells can't start one here.
        for windows in [Choice::WindowsPowerShell, Choice::GitBash, Choice::CommandPrompt, Choice::PowerShell7] {
            assert!(matches!(state(windows, &linux()), Readiness::CantUse(_)), "{windows:?}");
            assert!(launch(&plain(windows), &linux()).is_err(), "{windows:?}");
        }
    }
    #[cfg(not(windows))]
    #[test]
    fn linux_shells_are_found_from_etc_shells_and_custom_ones_need_a_full_executable_path() {
        use std::os::unix::fs::PermissionsExt;
        let root = std::env::temp_dir().join(format!("rocket-linux-shells-{}", std::process::id()));
        std::fs::create_dir_all(root.join("bin/folder-shell")).unwrap();
        let shell = root.join("bin/rocket-test-shell");
        std::fs::write(&shell, b"#!/bin/sh\n").unwrap();
        let listed = format!("# /etc/shells\n/nowhere/rocket-test-shell\n{}/bin/folder-shell\n  {}\n", root.display(), shell.display());
        assert_eq!(installed_shell("rocket-test-shell", Some(&listed)), Some(shell.clone()), "the first file listed");
        assert_eq!(installed_shell("folder-shell", Some(&listed)), None, "folders don't count");
        assert_eq!(installed_shell("rocket-test-shell", None), None, "nothing in /usr/bin");

        let custom = |program: &str| Settings { choice: Choice::Custom, program: program.into(), arguments: String::new() };
        assert!(matches!(readiness(&custom("zsh"), &linux(), None), Readiness::CantUse(_)), "a bare name is looked up nowhere");
        assert_eq!(readiness(&custom(&shell.to_string_lossy()), &linux(), None), Readiness::Checking, "no .exe needed");
        assert!(matches!(check_program(&shell), Status::Refused(_)), "not executable yet");
        std::fs::set_permissions(&shell, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(check_program(&shell), Status::Exists);
        assert!(matches!(check_program(&root.join("bin")), Status::Refused(_)));
        assert_eq!(check_program(&root.join("absent")), Status::MissingParent);
        // Custom starts from the shell you had.
        let mut state = State { located: linux(), ..Default::default() };
        state.editing.choice = Choice::Bash;
        state.choose(Choice::Custom);
        assert_eq!((state.editing.program.as_str(), state.editing.arguments.as_str()), ("/usr/bin/bash", ""));
        std::fs::remove_dir_all(root).unwrap();
    }
}
