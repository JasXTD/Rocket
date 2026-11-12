//! Commit message drafts with Claude Code or Codex in the Changes composer. Off until
//! turned on in Preferences; the first draft with each CLI explains what is sent and asks
//! first. A draft only fills the editable message fields: nothing is staged or committed.
use crate::{
    i18n::{tr, trf},
    app::*,
    backend::{Backend, user_error},
    pages,
    style::*,
};
use eframe::egui::*;
use rocket_git::*;
use std::{collections::HashMap, path::PathBuf};

/// Where Anthropic describes how Claude Code data is used and kept.
pub const DATA_USAGE_URL: &str = "https://code.claude.com/docs/en/data-usage";
/// Where OpenAI describes how Codex usage follows the signed-in account's data settings.
pub const CODEX_DATA_USAGE_URL: &str = "https://developers.openai.com/codex/auth";

/// Saved with the other preferences.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    /// Shows the Draft button in the composer. Off until turned on.
    pub enabled: bool,
    /// The CLI that drafts: Claude Code or Codex.
    pub provider: DraftProvider,
    /// Claude Code's first-use explanation was accepted with an explicit click.
    pub notice_accepted: bool,
    /// Codex's first-use explanation, asked separately.
    pub codex_notice_accepted: bool,
    pub style: DraftStyle,
    /// Claude Code's claude.exe; empty uses the native installer's location.
    pub executable: String,
    /// Codex's codex.exe; empty uses the first Codex found.
    pub codex_executable: String,
    pub language: String,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            enabled: false,
            provider: DraftProvider::Claude,
            notice_accepted: false,
            codex_notice_accepted: false,
            style: DraftStyle::Plain,
            executable: String::new(),
            codex_executable: String::new(),
            language: "English".into(),
        }
    }
}
impl Settings {
    /// The selected CLI's program setting.
    pub fn program_setting(&self) -> &str {
        match self.provider {
            DraftProvider::Claude => &self.executable,
            DraftProvider::Codex => &self.codex_executable,
        }
    }
    pub(crate) fn program_setting_mut(&mut self) -> &mut String {
        match self.provider {
            DraftProvider::Claude => &mut self.executable,
            DraftProvider::Codex => &mut self.codex_executable,
        }
    }
    /// Whether the selected CLI's first-use explanation was accepted.
    pub fn accepted(&self) -> bool {
        match self.provider {
            DraftProvider::Claude => self.notice_accepted,
            DraftProvider::Codex => self.codex_notice_accepted,
        }
    }
    fn accept(&mut self) {
        match self.provider {
            DraftProvider::Claude => self.notice_accepted = true,
            DraftProvider::Codex => self.codex_notice_accepted = true,
        }
    }
    pub fn request(&self) -> DraftRequest {
        let executable = self.program_setting().trim();
        DraftRequest {
            provider: self.provider,
            executable: (!executable.is_empty()).then(|| PathBuf::from(executable)),
            style: self.style,
            language: self.language.clone(),
        }
    }
    /// The program a draft would start, as configured (not yet checked).
    pub fn program(&self) -> Option<PathBuf> {
        self.request()
            .executable
            .or_else(|| self.provider.default_program())
    }
    /// A CLI's program setting, whichever CLI drafts commit messages.
    pub(crate) fn program_setting_of(&self, provider: DraftProvider) -> &str {
        match provider {
            DraftProvider::Claude => &self.executable,
            DraftProvider::Codex => &self.codex_executable,
        }
    }
    pub(crate) fn program_setting_of_mut(&mut self, provider: DraftProvider) -> &mut String {
        match provider {
            DraftProvider::Claude => &mut self.executable,
            DraftProvider::Codex => &mut self.codex_executable,
        }
    }
    /// A request for other AI features, whichever CLI drafts commit messages: that
    /// CLI's program and the language set here.
    pub fn request_for(&self, provider: DraftProvider) -> DraftRequest {
        let executable = self.program_setting_of(provider).trim();
        DraftRequest {
            provider,
            executable: (!executable.is_empty()).then(|| PathBuf::from(executable)),
            style: DraftStyle::Plain,
            language: self.language.clone(),
        }
    }
    /// The program [`Self::request_for`] would start (not yet checked).
    pub fn program_for(&self, provider: DraftProvider) -> Option<PathBuf> {
        self.request_for(provider)
            .executable
            .or_else(|| provider.default_program())
    }
}

/// What a CLI's program setting points at.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ProgramCheck {
    Found(String),
    Missing,
    Problem(String),
}

/// How long a check of a program stands, so one installed or removed while Rocket runs
/// still shows, without a look at the disk every frame.
const PROGRAM_CHECK: std::time::Duration = std::time::Duration::from_secs(2);

/// Whether a CLI's program is there, checked when its path changes and every few seconds.
pub(crate) fn check_program(ctx: &Context, settings: &Settings, provider: DraftProvider) -> ProgramCheck {
    let id = Id::unique(("ai-program-check", provider.code()));
    let key = settings.program_setting_of(provider).trim().to_owned();
    let cached = ctx
        .data(|d| d.get_temp::<(String, std::time::Instant, ProgramCheck)>(id))
        .filter(|(k, at, _)| *k == key && at.elapsed() < PROGRAM_CHECK);
    if let Some((_, _, check)) = cached {
        return check;
    }
    let check = match provider.executable(settings.request_for(provider).executable.as_deref()) {
        Ok(path) => ProgramCheck::Found(crate::views::display_path(&path)),
        Err(Error::CommitDraft { failure: DraftFailure::ClaudeMissing | DraftFailure::CodexMissing, .. }) => {
            ProgramCheck::Missing
        }
        Err(Error::CommitDraft { failure, .. }) => ProgramCheck::Problem(failure.guidance(provider).to_owned()),
        Err(other) => ProgramCheck::Problem(user_error(&other)),
    };
    ctx.data_mut(|d| d.insert_temp(id, (key, std::time::Instant::now(), check.clone())));
    check
}

/// The CLI's name on the composer's button.
fn short_name(provider: DraftProvider) -> &'static str {
    match provider {
        DraftProvider::Claude => "Claude",
        DraftProvider::Codex => "Codex",
    }
}

/// One tab's drafting state, kept with its composer.
#[derive(Default)]
pub struct Draft {
    task: Option<ReadTask<CommitDraft>>,
    /// The CLI of the running or last draft; follows Preferences while idle.
    pub provider: DraftProvider,
    /// Set by the composer's button; the next poll starts the draft or asks first.
    pub requested: bool,
    /// The message a draft replaced, offered back until the draft is edited.
    pub previous: Option<String>,
    drafted: Option<String>,
    pub notice: Option<String>,
    pub error: Option<String>,
    /// The failure code of the last draft, for automation.
    pub failure: Option<&'static str>,
    pub scope: Option<DraftScope>,
    /// The Preferences switch, mirrored here for the composer.
    pub available: bool,
}
impl Draft {
    pub fn running(&self) -> bool {
        self.task.is_some()
    }
    pub fn cancel(&self) {
        if let Some(task) = &self.task {
            task.cancel();
        }
    }
    fn start(&mut self, backend: Option<&Backend>, path: PathBuf, settings: &Settings) {
        if self.task.is_some() {
            return;
        }
        self.notice = None;
        self.error = None;
        self.failure = None;
        let Some(backend) = backend.cloned() else {
            self.error = Some("Rocket is still starting; try again in a moment.".into());
            return;
        };
        self.provider = settings.provider;
        let request = settings.request();
        match ReadTask::spawn(move |cancel| backend.commit_draft(&path, &request, &cancel)) {
            Ok(task) => self.task = Some(task),
            Err(error) => self.error = Some(user_error(&error)),
        }
    }
}

/// What the composer says about a finished draft: a short line, never the message.
fn scope_notice(scope: &DraftScope, provider: DraftProvider) -> String {
    let mut notice = format!(
        "Drafted by {} from {} staged file{}. Review it before committing.",
        provider.label(),
        scope.files,
        if scope.files == 1 { "" } else { "s" }
    );
    if scope.shortened() {
        notice.push_str(&format!(
            " The diff was shortened to {} KiB of {} KiB; {} file{} named only.",
            scope.sent_bytes.div_ceil(1024),
            scope.diff_bytes.div_ceil(1024),
            scope.omitted_files,
            if scope.omitted_files == 1 {
                " was"
            } else {
                "s were"
            }
        ));
    }
    if scope.binary_files > 0 {
        notice.push_str(&format!(
            " {} binary file{} named only.",
            scope.binary_files,
            if scope.binary_files == 1 {
                " was"
            } else {
                "s were"
            }
        ));
    }
    notice
}

impl Rocket {
    /// Each frame: mirror the Preferences switch, apply finished drafts, and start a
    /// requested one (after the first-use explanation, if it was not accepted yet).
    pub(crate) fn poll_commit_drafts(&mut self) {
        let backend = self.backend.clone();
        let mut ask = None;
        for tab in &mut self.tabs {
            let d = &mut tab.composer.draft;
            d.available = self.drafting.enabled;
            if let Some(result) = d.task.as_mut().and_then(ReadTask::try_finish) {
                d.task = None;
                match result {
                    Ok(draft) => {
                        let before = std::mem::replace(&mut tab.draft, draft.message.clone());
                        d.previous = (!before.trim().is_empty() && before != draft.message)
                            .then_some(before);
                        d.drafted = Some(draft.message);
                        d.notice = Some(scope_notice(&draft.scope, d.provider));
                        d.scope = Some(draft.scope);
                    }
                    Err(Error::Cancelled) => {
                        d.notice = Some("Draft cancelled; the message was not changed.".into());
                    }
                    Err(error) => {
                        d.failure = match &error {
                            Error::CommitDraft { failure, .. } => Some(failure.code()),
                            _ => None,
                        };
                        d.error = Some(user_error(&error));
                    }
                }
            }
            // Editing the drafted text ends the offer to restore the earlier message.
            if d.drafted.as_ref().is_some_and(|m| *m != tab.draft) {
                d.drafted = None;
                d.previous = None;
            }
            if !d.running() {
                d.provider = self.drafting.provider;
            }
            if std::mem::take(&mut d.requested) && self.drafting.enabled {
                if self.drafting.accepted() {
                    d.start(backend.as_ref(), tab.path.clone(), &self.drafting);
                } else {
                    ask = Some(tab.id);
                }
            }
        }
        if let Some(id) = ask
            && self.dialog.is_none()
        {
            self.show_dialog(DialogKind::DraftNotice(id));
        }
    }

    /// The first-use explanation. Its button is the explicit consent; nothing is sent
    /// before it is clicked, and closing it sends nothing.
    pub(crate) fn draft_notice_dialog(&mut self, ctx: &Context) {
        let Some(dialog) = self.dialog.take() else {
            return;
        };
        let DialogKind::DraftNotice(owner) = dialog.kind else {
            self.dialog = Some(dialog);
            return;
        };
        let Some(tab) = self.tabs.iter().find(|t| t.id == owner) else {
            return;
        };
        let staged = tab
            .snapshot
            .as_ref()
            .and_then(|s| s.status.as_ref())
            .map_or(0, |s| {
                s.entries
                    .iter()
                    .filter(|e| e.staged() && e.kind != EntryKind::Unmerged)
                    .count()
            });
        let provider = self.drafting.provider;
        let codex = provider == DraftProvider::Codex;
        let program = self.drafting.program().map_or_else(
            || format!("{}.exe (not found)", provider.code()),
            |p| p.display().to_string(),
        );
        let title = format!("Draft commit messages with {}", provider.label());
        let mut send = false;
        let response = pages::review(
            ctx,
            "rocket-draft-notice",
            pages::ReviewHeader {
                image: Icon::Commit,
                title: &title,
                subtitle: "Read what is sent before the first draft.",
                danger: false,
            },
            &mut self.frame_widgets,
            "dialog-draft-notice-scroll",
            true,
            |ui, part, widgets| match part {
                pages::Part::Body => {
                    pages::caption(ui, "What is sent");
                    pages::well(ui, |ui| {
                        pages::field(ui, "Program", &program, true);
                        pages::field(
                            ui,
                            "Staged changes",
                            &format!(
                                "{staged} staged file{} of {}: their names and diff, up to {} KiB",
                                if staged == 1 { "" } else { "s" },
                                tab.title,
                                DRAFT_DIFF_LIMIT / 1024
                            ),
                            false,
                        );
                        pages::field(
                            ui,
                            "Also",
                            "Rocket's fixed instructions, the style and the language",
                            false,
                        );
                    });
                    if codex {
                        pages::note(
                            ui,
                            "Rocket runs Codex on this computer and gives it the staged changes. Codex sends them to OpenAI to write the draft, under the ChatGPT or OpenAI account Codex is signed in with and OpenAI's terms. A larger diff is shortened, binary files are named only, and unstaged or untracked files are never sent.",
                        );
                        pages::note(
                            ui,
                            "Codex runs read-only from an empty folder, without your Codex settings (config.toml), rules, MCP servers, apps, plugins, hooks, web search or a saved session, and with its shell and other tools switched off, so it cannot run commands, read other files or change anything. Codex still adds your own global instructions (AGENTS.md in your Codex folder), if you have any, as it does in every Codex run.",
                        );
                        pages::note(
                            ui,
                            "Nothing is committed: the draft goes into the message fields for you to edit. Rocket's logs record only sizes and timings.",
                        );
                        pages::field(ui, "Data use", CODEX_DATA_USAGE_URL, true);
                        pages::callout(
                            ui,
                            BLUE(),
                            "This is asked once for Codex, separately from Claude Code. Afterwards each Draft with Codex click sends the staged changes; turn drafting off in Preferences at any time.",
                        );
                    } else {
                        pages::note(
                            ui,
                            "Rocket runs Claude Code on this computer and gives it the staged changes. Claude Code sends them to Anthropic to write the draft, under your Claude account and Anthropic's terms. A larger diff is shortened, binary files are named only, and unstaged or untracked files are never sent.",
                        );
                        pages::note(
                            ui,
                            "Claude Code runs without tools, settings, hooks, MCP servers or a saved session, so it cannot run commands, read other files or change anything. Nothing is committed: the draft goes into the message fields for you to edit. Rocket's logs record only sizes and timings.",
                        );
                        pages::field(ui, "Data use", DATA_USAGE_URL, true);
                        pages::callout(
                            ui,
                            BLUE(),
                            "This is asked once. Afterwards each Draft with Claude click sends the staged changes; turn drafting off in Preferences at any time.",
                        );
                    }
                }
                pages::Part::Footer => {
                    let label = format!("Send staged changes to {}", short_name(provider));
                    let r = pages::primary(ui, staged > 0, &label);
                    pages::control(widgets, "dialog-submit", &r, ui.clip_rect());
                    send = r.clicked();
                }
            },
        );
        if send {
            self.drafting.accept();
            let backend = self.backend.clone();
            if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == owner) {
                tab.composer
                    .draft
                    .start(backend.as_ref(), tab.path.clone(), &self.drafting);
            }
        } else if !response.should_close() {
            self.dialog = Some(dialog);
        }
    }
}

/// The composer's Draft with Claude or Draft with Codex button, or the running draft
/// and its Cancel.
pub(crate) fn composer_button(
    ui: &mut Ui,
    d: &mut Draft,
    enabled: bool,
    widgets: &mut HashMap<String, UiControl>,
) {
    if !d.available {
        return;
    }
    let name = d.provider.label();
    if d.running() {
        ui.spinner();
        let r = ui
            .add(crate::style::quiet(Button::new(RichText::new("Stop drafting").size(12.5))))
            .on_hover_text(format!("Stop {name}; the message stays as it was"));
        widgets.insert("composer-draft-cancel".into(), (&r).into());
        if r.clicked() {
            d.cancel();
        }
        return;
    }
    let label = format!("Draft with {}", short_name(d.provider));
    let r = ui
        .add_enabled(
            enabled,
            crate::style::quiet(Button::new(RichText::new(label).size(12.5))),
        )
        .on_hover_text(format!(
            "Write a draft of the message from the staged changes with {name}"
        ))
        .on_disabled_hover_text("Stage changes to draft a message from them");
    widgets.insert("composer-draft".into(), (&r).into());
    if r.clicked() {
        d.requested = true;
    }
}

/// One status line under the composer for the last draft, with Restore when a draft
/// replaced `message`. Returns `false` when there is nothing to show.
pub(crate) fn composer_status(
    ui: &mut Ui,
    d: &mut Draft,
    message: &mut String,
    widgets: &mut HashMap<String, UiControl>,
) -> bool {
    if d.running() {
        ui.label(
            RichText::new(format!(
                "{} is drafting from the staged changes…",
                d.provider.label()
            ))
            .size(11.5)
            .color(FAINT()),
        );
        return true;
    }
    let (text, color, full) = match (&d.error, &d.notice) {
        (Some(error), _) => (
            format!("Draft failed: {}", error.lines().next().unwrap_or_default()),
            AMBER(),
            error.clone(),
        ),
        (None, Some(notice)) => (notice.clone(), FAINT(), notice.clone()),
        (None, None) => return false,
    };
    let mut restore = None;
    ui.horizontal(|ui| {
        if d.previous.is_some() {
            let r = ui.add(crate::style::quiet(Button::new(
                RichText::new("Restore previous").size(11.5),
            )));
            widgets.insert("composer-draft-restore".into(), (&r).into());
            if r.clicked() {
                restore = d.previous.take();
            }
        }
        ui.add(Label::new(RichText::new(text).size(11.5).color(color)).truncate())
            .on_hover_text(full);
    });
    if let Some(previous) = restore {
        d.drafted = None;
        d.notice = Some("Restored the message from before the draft.".into());
        *message = previous;
    }
    true
}

/// The AI drafts card in Preferences → Commit messages. Returns whether Browse was
/// clicked for the program. What a draft sends is spelled out in the guide and in the
/// first-use explanation of each CLI.
pub(crate) fn preferences(
    ui: &mut Ui,
    settings: &mut Settings,
    widgets: &mut HashMap<String, UiControl>,
) -> bool {
    use crate::preferences::{choices, divider, field_row, status_line, switch_row};
    let hint = if settings.enabled {
        tr("Drafts are never committed for you.")
    } else {
        tr("Off: nothing is sent anywhere.")
    };
    switch_row(
        ui,
        widgets,
        "pref-ai-enabled",
        &mut settings.enabled,
        tr("Show a Draft button in the commit composer"),
        hint,
        "",
    );
    if !settings.enabled {
        return false;
    }
    divider(ui);
    field_row(ui, tr("Draft with"), |ui| {
        let items = [
            ("Claude Code · Anthropic", settings.provider == DraftProvider::Claude),
            ("Codex · OpenAI", settings.provider == DraftProvider::Codex),
        ];
        let ids = ["pref-ai-provider-claude".to_owned(), "pref-ai-provider-codex".to_owned()];
        if let Some(i) = choices(ui, widgets, &items, &ids) {
            settings.provider = [DraftProvider::Claude, DraftProvider::Codex][i];
        }
    });
    let provider = settings.provider;
    let mut browse = false;
    field_row(ui, tr("Program"), |ui| {
        let hint = match provider {
            DraftProvider::Claude => {
                default_claude_path().map_or_else(String::new, |p| p.display().to_string())
            }
            DraftProvider::Codex => tr("Found automatically when empty").into(),
        };
        ui.horizontal(|ui| {
            let r = ui.add(
                TextEdit::singleline(settings.program_setting_mut())
                    .hint_text(&hint)
                    .font(FontId::monospace(12.5))
                    .char_limit(32 * 1024)
                    .desired_width((ui.available_width() - 84.0).max(80.0)),
            );
            pages::control(widgets, "pref-ai-executable", &r, ui.clip_rect());
            let b = ui.button(tr("Browse"));
            pages::control(widgets, "pref-ai-browse", &b, ui.clip_rect());
            browse = b.clicked();
        });
        match check_program(ui.ctx(), settings, provider) {
            ProgramCheck::Found(path) => status_line(ui, GREEN(), &trf!("Found {path}", path)),
            ProgramCheck::Missing => {
                let missing = match provider {
                    DraftProvider::Claude => tr("Claude Code was not found. Install it or choose its executable in Preferences."),
                    DraftProvider::Codex => tr("Codex was not found. Install the Codex CLI or app, or choose its executable in Preferences."),
                };
                status_line(ui, AMBER(), missing);
            }
            ProgramCheck::Problem(problem) => status_line(ui, AMBER(), &problem),
        }
    });
    field_row(ui, tr("Message style"), |ui| {
        let items = [
            (tr("Plain summary and body"), settings.style == DraftStyle::Plain),
            (tr("Conventional Commits"), settings.style == DraftStyle::Conventional),
        ];
        let ids = ["pref-ai-style-plain".to_owned(), "pref-ai-style-conventional".to_owned()];
        if let Some(i) = choices(ui, widgets, &items, &ids) {
            settings.style = [DraftStyle::Plain, DraftStyle::Conventional][i];
        }
    });
    field_row(ui, tr("Language"), |ui| {
        let r = ui.add(
            TextEdit::singleline(&mut settings.language)
                .hint_text("English")
                .char_limit(DRAFT_LANGUAGE_LIMIT)
                .desired_width(240.0),
        );
        pages::control(widgets, "pref-ai-language", &r, ui.clip_rect());
        if settings.request().validate().is_err() {
            status_line(ui, AMBER(), tr("Enter a language name using letters, spaces and hyphens, up to 64 characters."));
        }
    });
    divider(ui);
    Frame::new()
        .inner_margin(Margin::symmetric(16, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            let text = match provider {
                DraftProvider::Claude => {
                    tr("Each draft sends your staged changes to Anthropic through Claude Code. The first draft explains what is sent and asks first.")
                }
                DraftProvider::Codex => {
                    tr("Each draft sends your staged changes to OpenAI through Codex. The first draft explains what is sent and asks first.")
                }
            };
            ui.add(Label::new(RichText::new(text).size(12.0).color(HINT())).wrap());
        });
    browse
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notices_report_shortened_and_binary_parts_only() {
        let mut scope = DraftScope {
            files: 1,
            binary_files: 0,
            omitted_files: 0,
            diff_bytes: 10,
            sent_bytes: 10,
        };
        assert_eq!(
            scope_notice(&scope, DraftProvider::Claude),
            "Drafted by Claude Code from 1 staged file. Review it before committing."
        );
        scope.files = 40;
        scope.binary_files = 2;
        scope.omitted_files = 3;
        scope.diff_bytes = 300 * 1024;
        scope.sent_bytes = 128 * 1024;
        assert_eq!(
            scope_notice(&scope, DraftProvider::Codex),
            "Drafted by Codex from 40 staged files. Review it before committing. The diff was shortened to 128 KiB of 300 KiB; 3 files were named only. 2 binary files were named only."
        );
    }

    #[test]
    fn settings_default_off_and_map_to_requests() {
        let settings = Settings::default();
        assert!(!settings.enabled && !settings.notice_accepted && !settings.codex_notice_accepted);
        assert_eq!(settings.provider, DraftProvider::Claude);
        let request = settings.request();
        assert_eq!(request.executable, None);
        assert_eq!(request.language, "English");
        let mut custom = Settings {
            executable: "  C:\\Tools 文\\claude.exe ".into(),
            codex_executable: "D:\\Codex\\codex.exe".into(),
            notice_accepted: true,
            style: DraftStyle::Conventional,
            ..settings
        };
        assert_eq!(
            custom.request().executable,
            Some(PathBuf::from("C:\\Tools 文\\claude.exe"))
        );
        assert_eq!(custom.program(), custom.request().executable);
        assert!(custom.accepted());
        // Codex has its own program and its own first-use consent.
        custom.provider = DraftProvider::Codex;
        let request = custom.request();
        assert_eq!(request.provider, DraftProvider::Codex);
        assert_eq!(
            request.executable,
            Some(PathBuf::from("D:\\Codex\\codex.exe"))
        );
        assert!(!custom.accepted());
        custom.accept();
        assert!(custom.codex_notice_accepted && custom.accepted());
    }
}

/// The composer and the first-use dialog with a fake Claude Code on a real repository.
#[cfg(all(test, windows))]
mod flow_tests {
    use super::Settings;
    use crate::{
        app::{DialogKind, View},
        batch::flow_tests::{click, enabled, frame, open, wait},
        test_common as common,
    };
    use eframe::egui::Context;
    use rocket_git::DraftProvider;
    use std::{fs, path::PathBuf, process::Command};

    const DRAFT: &str =
        "Add PRIVATE_DRAFT_SUBJECT\n\nPRIVATE_DRAFT_BODY explains the staged change.";

    fn stub(f: &common::Fixture) -> PathBuf {
        stub_as(f, "claude.exe")
    }
    /// Built as codex.exe, the fake answers as Codex.
    fn stub_as(f: &common::Fixture, name: &str) -> PathBuf {
        let folder = f.root.join(format!("{name} stub"));
        fs::create_dir_all(&folder).unwrap();
        let exe = folder.join(name);
        let source = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../rocket-git/examples/claude_cli_fixture.rs");
        let output = Command::new("rustc")
            .args(["--edition=2024", "--crate-name=claude_cli_fixture"])
            .arg(source)
            .arg("-o")
            .arg(&exe)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        exe
    }
    fn sent(exe: &std::path::Path) -> usize {
        (1..)
            .take_while(|n| exe.with_file_name(format!("request-{n}.txt")).exists())
            .count()
    }

    #[test]
    fn drafting_asks_first_fills_the_message_and_can_be_undone_or_cancelled() {
        common::isolated(
            "ROCKET_DRAFT_UI_CASE",
            "commit_draft::flow_tests::drafting_asks_first_fills_the_message_and_can_be_undone_or_cancelled",
            || {
                let f = common::Fixture::new();
                f.write("app.txt", "base\n");
                f.commit("base");
                f.write("app.txt", "staged change\n");
                f.run(&["add", "app.txt"]);
                let exe = stub(&f);
                let ctx = Context::default();
                let mut app = open(&ctx, &f);
                app.tabs[0].view = View::Changes;
                frame(&ctx, &mut app, vec![]);
                // Off by default: no button.
                assert!(!app.frame_widgets.contains_key("composer-draft"));
                app.drafting = Settings {
                    enabled: true,
                    executable: exe.display().to_string(),
                    ..Settings::default()
                };
                app.tabs[0].draft = "typed by hand".into();
                app.poll(&ctx);
                // The first draft asks first; closing it with × sends nothing.
                click(&ctx, &mut app, "composer-draft");
                app.poll(&ctx);
                frame(&ctx, &mut app, vec![]);
                assert!(matches!(
                    app.dialog.as_ref().map(|d| &d.kind),
                    Some(DialogKind::DraftNotice(_))
                ));
                click(&ctx, &mut app, "dialog-cancel");
                assert!(app.dialog.is_none() && !app.drafting.notice_accepted);
                assert_eq!(sent(&exe), 0);
                // Accepting sends, and the draft fills the fields; nothing is committed.
                let head = f.run(&["rev-parse", "HEAD"]);
                click(&ctx, &mut app, "composer-draft");
                app.poll(&ctx);
                click(&ctx, &mut app, "dialog-submit");
                assert!(app.drafting.notice_accepted);
                wait(&ctx, &mut app);
                app.poll(&ctx);
                assert_eq!(app.tabs[0].draft, DRAFT);
                assert_eq!(sent(&exe), 1);
                assert_eq!(f.run(&["rev-parse", "HEAD"]), head);
                // Restore brings the hand-typed message back.
                click(&ctx, &mut app, "composer-draft-restore");
                assert_eq!(app.tabs[0].draft, "typed by hand");
                // Later drafts do not ask again. A slow one keeps Commit disabled and
                // Cancel leaves the message as it was.
                fs::write(exe.with_file_name("mode"), "slow").unwrap();
                click(&ctx, &mut app, "composer-draft");
                app.poll(&ctx);
                assert!(app.dialog.is_none());
                assert!(app.tabs[0].composer.draft.running());
                assert!(!enabled(&ctx, &mut app, "commit"));
                click(&ctx, &mut app, "composer-draft-cancel");
                wait(&ctx, &mut app);
                app.poll(&ctx);
                assert_eq!(app.tabs[0].draft, "typed by hand");
                assert_eq!(
                    app.tabs[0].composer.draft.notice.as_deref(),
                    Some("Draft cancelled; the message was not changed.")
                );
                // A failure is reported with its code, and the message is kept.
                fs::write(exe.with_file_name("mode"), "signin").unwrap();
                click(&ctx, &mut app, "composer-draft");
                // The click requests the draft; the next poll starts it.
                app.poll(&ctx);
                assert!(app.tabs[0].composer.draft.running());
                wait(&ctx, &mut app);
                app.poll(&ctx);
                assert_eq!(
                    app.tabs[0].composer.draft.failure,
                    Some("draft_sign_in_required")
                );
                assert_eq!(app.tabs[0].draft, "typed by hand");
            },
        );
    }

    #[test]
    fn codex_asks_separately_from_claude_and_fills_the_message() {
        common::isolated(
            "ROCKET_DRAFT_UI_CASE",
            "commit_draft::flow_tests::codex_asks_separately_from_claude_and_fills_the_message",
            || {
                let f = common::Fixture::new();
                f.write("app.txt", "base\n");
                f.commit("base");
                f.write("app.txt", "staged change\n");
                f.run(&["add", "app.txt"]);
                let exe = stub_as(&f, "codex.exe");
                let ctx = Context::default();
                let mut app = open(&ctx, &f);
                app.tabs[0].view = View::Changes;
                frame(&ctx, &mut app, vec![]);
                // Claude Code's explanation was accepted earlier; Codex still asks first.
                app.drafting = Settings {
                    enabled: true,
                    notice_accepted: true,
                    provider: DraftProvider::Codex,
                    codex_executable: exe.display().to_string(),
                    ..Settings::default()
                };
                app.poll(&ctx);
                assert_eq!(app.tabs[0].composer.draft.provider, DraftProvider::Codex);
                click(&ctx, &mut app, "composer-draft");
                app.poll(&ctx);
                frame(&ctx, &mut app, vec![]);
                assert!(matches!(
                    app.dialog.as_ref().map(|d| &d.kind),
                    Some(DialogKind::DraftNotice(_))
                ));
                click(&ctx, &mut app, "dialog-cancel");
                assert!(app.dialog.is_none() && !app.drafting.codex_notice_accepted);
                assert_eq!(sent(&exe), 0);
                // Accepting sends to Codex; the draft fills the fields, nothing is committed.
                let head = f.run(&["rev-parse", "HEAD"]);
                click(&ctx, &mut app, "composer-draft");
                app.poll(&ctx);
                click(&ctx, &mut app, "dialog-submit");
                assert!(app.drafting.codex_notice_accepted);
                wait(&ctx, &mut app);
                app.poll(&ctx);
                assert_eq!(app.tabs[0].draft, DRAFT);
                assert_eq!(sent(&exe), 1);
                assert_eq!(f.run(&["rev-parse", "HEAD"]), head);
                let notice = app.tabs[0]
                    .composer
                    .draft
                    .notice
                    .clone()
                    .unwrap_or_default();
                assert!(
                    notice.starts_with("Drafted by Codex from 1 staged file"),
                    "{notice}"
                );
                // A typed failure names Codex and keeps the message.
                fs::write(exe.with_file_name("mode"), "signin").unwrap();
                click(&ctx, &mut app, "composer-draft");
                app.poll(&ctx);
                assert!(app.tabs[0].composer.draft.running());
                wait(&ctx, &mut app);
                app.poll(&ctx);
                let draft = &app.tabs[0].composer.draft;
                assert_eq!(draft.failure, Some("draft_sign_in_required"));
                let error = draft.error.clone().unwrap_or_default();
                assert!(error.contains("Codex is not signed in"), "{error}");
                assert_eq!(app.tabs[0].draft, DRAFT);
            },
        );
    }
}
