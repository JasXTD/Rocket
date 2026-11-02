//! Git Flow: Initialize, Start and Finish as reviewed dialogs. Every input change
//! drops the review; the write revalidates it under the repository's write gate.
use crate::{
    app::*,
    backend::{Action, Outcome, user_error},
    pages,
    style::*,
};
use eframe::egui::*;
use rocket_git::*;
use std::sync::Arc;

/// What the Git Flow dialog was opened for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Request {
    Init,
    Start(GitFlowKind),
    /// Finish a git-flow branch; the current one is chosen first.
    Finish,
}
impl Request {
    /// A fixed name for automation state.
    pub fn code(self) -> &'static str {
        match self {
            Self::Init => "init",
            Self::Start(GitFlowKind::Feature) => "start-feature",
            Self::Start(GitFlowKind::Release) => "start-release",
            Self::Start(GitFlowKind::Hotfix) => "start-hotfix",
            Self::Start(GitFlowKind::Support) => "start-support",
            Self::Finish => "finish",
        }
    }
}

#[derive(Clone, Debug)]
pub enum Preview {
    Init(Arc<GitFlowInitPreview>),
    Start(Arc<GitFlowStartPreview>),
    Finish(Arc<GitFlowFinishPreview>),
}

pub struct DialogState {
    pub request: Request,
    /// git-flow's settings and branches, read when the dialog opens.
    pub state: Option<GitFlowState>,
    state_task: Option<ReadTask<GitFlowState>>,
    /// Initialize: the settings being edited.
    pub config: GitFlowConfig,
    /// Start: the feature name or version. Finish: the tag message.
    pub name: String,
    /// Start a support branch: its base revision.
    pub base: String,
    /// Finish: the chosen branch and options.
    pub branch: Option<String>,
    pub options: GitFlowFinishOptions,
    pub preview: Option<Preview>,
    task: Option<ReadTask<Preview>>,
    pub applying: bool,
}
impl DialogState {
    fn new(request: Request) -> Self {
        Self {
            request,
            state: None,
            state_task: None,
            config: GitFlowConfig::defaults("main"),
            name: String::new(),
            base: String::new(),
            branch: None,
            options: GitFlowFinishOptions::default(),
            preview: None,
            task: None,
            applying: false,
        }
    }
    pub fn running(&self) -> bool {
        self.state_task.is_some() || self.task.is_some()
    }
    pub fn loading_state(&self) -> bool {
        self.state_task.is_some()
    }
    pub fn reviewing(&self) -> bool {
        self.task.is_some()
    }
    fn cancel(&mut self) {
        if let Some(task) = &self.task {
            task.cancel();
        }
        if let Some(task) = &self.state_task {
            task.cancel();
        }
    }
    /// Branches Finish offers: features, releases and hotfixes.
    pub fn finishable(&self) -> Vec<&GitFlowBranch> {
        self.state.as_ref().map_or_else(Vec::new, |s| {
            s.branches.iter().filter(|b| b.kind.finishes()).collect()
        })
    }
    fn chosen(&self) -> Option<&GitFlowBranch> {
        let branch = self.branch.as_deref()?;
        self.finishable().into_iter().find(|b| b.branch == branch)
    }
}

impl Rocket {
    /// Read git-flow's settings and branches when its dialog opens.
    pub(crate) fn start_gitflow_read(&mut self) {
        let Some(dialog) = &mut self.dialog else {
            return;
        };
        let DialogKind::GitFlow(request) = dialog.kind else {
            return;
        };
        let state = dialog
            .gitflow
            .get_or_insert_with(|| DialogState::new(request));
        let path = self
            .tabs
            .iter()
            .find(|t| Some(t.id) == dialog.owner)
            .map(|t| t.path.clone());
        match (self.backend.clone(), path) {
            (Some(backend), Some(path)) => {
                match ReadTask::spawn(move |cancel| backend.gitflow_state(&path, &cancel)) {
                    Ok(task) => state.state_task = Some(task),
                    Err(e) => dialog.error = Some(user_error(&e)),
                }
            }
            _ => dialog.error = Some("Open a repository first.".into()),
        }
    }
    pub(crate) fn poll_gitflow(&mut self) {
        let Some(dialog) = &mut self.dialog else {
            return;
        };
        let Some(state) = &mut dialog.gitflow else {
            return;
        };
        if let Some(result) = state.state_task.as_mut().and_then(ReadTask::try_finish) {
            state.state_task = None;
            match result {
                Ok(read) => {
                    state.config = read.suggested();
                    state.branch = read
                        .current()
                        .filter(|b| b.kind.finishes())
                        .or_else(|| read.branches.iter().find(|b| b.kind.finishes()))
                        .map(|b| b.branch.clone());
                    state.state = Some(read);
                    if state.request == Request::Finish {
                        state.options.tag_message = default_message(state);
                    }
                    if matches!(state.request, Request::Start(GitFlowKind::Support)) {
                        state.base = state.config.master.clone();
                    }
                }
                Err(e) => dialog.error = Some(user_error(&e)),
            }
        }
        if let Some(result) = state.task.as_mut().and_then(ReadTask::try_finish) {
            state.task = None;
            match result {
                Ok(preview) => state.preview = Some(preview),
                Err(e) => dialog.error = Some(user_error(&e)),
            }
        }
    }
    fn start_gitflow_preview(&mut self) {
        let Some(dialog) = &mut self.dialog else {
            return;
        };
        let Some(state) = &mut dialog.gitflow else {
            return;
        };
        if state.applying {
            return;
        }
        state.preview = None;
        state.task = None;
        dialog.error = None;
        let (Some(backend), Some(path)) = (
            self.backend.clone(),
            self.tabs
                .iter()
                .find(|t| Some(t.id) == dialog.owner)
                .map(|t| t.path.clone()),
        ) else {
            dialog.error = Some("Open a repository first.".into());
            return;
        };
        let spawned = match state.request {
            Request::Init => {
                let config = state.config.clone();
                ReadTask::spawn(move |cancel| {
                    backend
                        .gitflow_init_preview(&path, &config, &cancel)
                        .map(|p| Preview::Init(Arc::new(p)))
                })
            }
            Request::Start(kind) => {
                let name = state.name.clone();
                let base = (kind == GitFlowKind::Support && !state.base.trim().is_empty())
                    .then(|| state.base.trim().to_owned());
                ReadTask::spawn(move |cancel| {
                    backend
                        .gitflow_start_preview(&path, kind, &name, base.as_deref(), &cancel)
                        .map(|p| Preview::Start(Arc::new(p)))
                })
            }
            Request::Finish => {
                let Some(branch) = state.branch.clone() else {
                    dialog.error = Some("Choose a branch to finish.".into());
                    return;
                };
                let options = state.options.clone();
                ReadTask::spawn(move |cancel| {
                    backend
                        .gitflow_finish_preview(&path, &branch, &options, &cancel)
                        .map(|p| Preview::Finish(Arc::new(p)))
                })
            }
        };
        match spawned {
            Ok(task) => state.task = Some(task),
            Err(e) => dialog.error = Some(user_error(&e)),
        }
    }
    pub(crate) fn finish_gitflow_dialog(&mut self, owner: Option<u64>, outcome: &Outcome) {
        let Some(dialog) = &mut self.dialog else {
            return;
        };
        let Some(state) = &mut dialog.gitflow else {
            return;
        };
        if !state.applying || dialog.owner != owner {
            return;
        }
        if outcome.command_succeeded || outcome.recovery_pending {
            self.dialog = None;
            if outcome.recovery_pending
                && let Some(tab) = self.tabs.iter_mut().find(|t| Some(t.id) == owner)
            {
                tab.view = View::Changes;
            }
        } else {
            state.applying = false;
            state.preview = None;
            dialog.error = Some(format!(
                "{}\n{}\nReview again before retrying.",
                outcome.summary,
                outcome.details.join("\n")
            ));
        }
    }
    pub(crate) fn cancel_gitflow_reads(&mut self) {
        if let Some(state) = self.dialog.as_mut().and_then(|d| d.gitflow.as_mut()) {
            state.cancel();
        }
    }
    pub(crate) fn gitflow_dialog(&mut self, ctx: &Context) {
        let Some(mut dialog) = self.dialog.take() else {
            return;
        };
        let DialogKind::GitFlow(request) = dialog.kind else {
            self.dialog = Some(dialog);
            return;
        };
        let state = dialog
            .gitflow
            .get_or_insert_with(|| DialogState::new(request));
        if !state.applying && dialog.owner != self.tab().map(|t| t.id) {
            return;
        }
        let waiting = state.running() || state.applying;
        let idle = self.write.is_none();
        let running = pages::Running::of(self.write.as_ref());
        let (mut dismiss, mut refresh, mut submit, mut cancel, mut changed) =
            (false, false, false, false, false);
        let (title, subtitle) = heading(request);
        let response = pages::review(
            ctx,
            "rocket-gitflow",
            pages::ReviewHeader {
                image: Icon::Branch,
                title: &title,
                subtitle,
                danger: false,
            },
            &mut self.frame_widgets,
            "dialog-gitflow-scroll",
            !state.applying,
            |ui, part, widgets| match part {
                pages::Part::Body => {
                    if let Some(error) = &dialog.error {
                        pages::callout(ui, RED(), error);
                    }
                    if state.loading_state() {
                        pages::waiting(ui, "Reading Git Flow settings and branches…");
                        return;
                    }
                    let initialized = state.state.as_ref().is_some_and(|s| s.config.is_some());
                    if request != Request::Init && !initialized {
                        pages::callout(ui, AMBER(), GitFlowFailure::NotInitialized.guidance());
                        return;
                    }
                    changed |= match request {
                        Request::Init => init_inputs(ui, state, !waiting, widgets),
                        Request::Start(kind) => start_inputs(ui, state, kind, !waiting, widgets),
                        Request::Finish => finish_inputs(ui, state, !waiting, widgets),
                    };
                    if changed {
                        state.preview = None;
                        dialog.error = None;
                    }
                    if state.reviewing() {
                        pages::waiting(ui, "Reviewing branches, checkout and settings…");
                    }
                    match &state.preview {
                        Some(Preview::Init(p)) => init_review(ui, p),
                        Some(Preview::Start(p)) => start_review(ui, p),
                        Some(Preview::Finish(p)) => finish_review(ui, p),
                        None if !waiting => pages::note(ui, "Review first."),
                        None => {}
                    }
                }
                pages::Part::Footer if state.applying => {
                    cancel = pages::running_footer(ui, widgets, &running, &[]);
                }
                pages::Part::Footer => {
                    let ready = idle
                        && !waiting
                        && match &state.preview {
                            Some(Preview::Init(p)) => !p.unchanged(),
                            Some(_) => true,
                            None => false,
                        };
                    let r = pages::primary(ui, ready, &submit_label(request));
                    pages::control(widgets, "dialog-submit", &r, ui.clip_rect());
                    submit = r.clicked();
                    let initialized = state.state.as_ref().is_some_and(|s| s.config.is_some());
                    let r = ui.add_enabled(
                        idle && !waiting && (initialized || request == Request::Init),
                        Button::new("Review"),
                    );
                    pages::control(widgets, "dialog-preview-refresh", &r, ui.clip_rect());
                    refresh = r.clicked();
                }
            },
        );
        dismiss |= response.should_close() && !state.applying;
        if cancel {
            self.cancel_write();
        }
        if submit && let (Some(owner), Some(preview)) = (dialog.owner, state.preview.clone()) {
            state.applying = true;
            self.execute(
                owner,
                match preview {
                    Preview::Init(p) => Action::GitFlowInit(p),
                    Preview::Start(p) => Action::GitFlowStart(p),
                    Preview::Finish(p) => Action::GitFlowFinish(p),
                },
            );
            if self.write.is_none() {
                state.applying = false;
                dialog.error = Some(
                    "Git Flow could not start. Wait for the repository and review again.".into(),
                );
            }
        }
        if dismiss {
            state.cancel();
        } else {
            self.dialog = Some(dialog);
            if refresh {
                self.start_gitflow_preview();
            }
        }
    }
}

fn heading(request: Request) -> (String, &'static str) {
    match request {
        Request::Init => (
            "Initialize Git Flow".into(),
            "Store git-flow's branch names and prefixes in this repository's Git config.",
        ),
        Request::Start(kind) => (
            format!("Start {}", kind.title()),
            match kind {
                GitFlowKind::Feature => "A feature branch from the development branch.",
                GitFlowKind::Release => "A release branch from the development branch.",
                GitFlowKind::Hotfix => "A hotfix branch from the production branch.",
                GitFlowKind::Support => "A long-lived support branch from a chosen commit.",
            },
        ),
        Request::Finish => (
            "Finish Git Flow branch".into(),
            "Review every merge, tag and deletion before anything runs.",
        ),
    }
}
fn submit_label(request: Request) -> String {
    match request {
        Request::Init => "Initialize".into(),
        Request::Start(kind) => format!("Start {}", kind.label()),
        Request::Finish => "Finish".into(),
    }
}
fn default_message(state: &DialogState) -> String {
    state
        .chosen()
        .filter(|b| b.kind.tagged())
        .map_or_else(String::new, |b| format!("{} {}", b.kind.title(), b.name))
}
fn short(id: &ObjectId) -> &str {
    &id.as_str()[..9]
}

fn text_row(
    ui: &mut Ui,
    label: &str,
    value: &mut String,
    hint: &str,
    key: &str,
    enabled: bool,
    widgets: &mut std::collections::HashMap<String, UiControl>,
) -> bool {
    ui.label(label);
    let r = ui.add_enabled(
        enabled,
        TextEdit::singleline(value)
            .hint_text(hint)
            .char_limit(GITFLOW_NAME_LIMIT)
            .desired_width(f32::INFINITY),
    );
    pages::control(widgets, key, &r, ui.clip_rect());
    r.changed()
}

fn init_inputs(
    ui: &mut Ui,
    state: &mut DialogState,
    enabled: bool,
    widgets: &mut std::collections::HashMap<String, UiControl>,
) -> bool {
    if state.state.as_ref().is_some_and(GitFlowState::incomplete) {
        pages::callout(
            ui,
            AMBER(),
            "Some gitflow.* settings exist, but not a complete setup. Review the values below.",
        );
    }
    let c = &mut state.config;
    let mut changed = false;
    pages::caption(ui, "Branches");
    changed |= text_row(
        ui,
        "Production releases",
        &mut c.master,
        "main",
        "dialog-gitflow-master",
        enabled,
        widgets,
    );
    changed |= text_row(
        ui,
        "Next release development",
        &mut c.develop,
        "develop",
        "dialog-gitflow-develop",
        enabled,
        widgets,
    );
    pages::caption(ui, "Prefixes");
    changed |= text_row(
        ui,
        "Feature branches",
        &mut c.feature,
        "feature/",
        "dialog-gitflow-feature",
        enabled,
        widgets,
    );
    changed |= text_row(
        ui,
        "Release branches",
        &mut c.release,
        "release/",
        "dialog-gitflow-release",
        enabled,
        widgets,
    );
    changed |= text_row(
        ui,
        "Hotfix branches",
        &mut c.hotfix,
        "hotfix/",
        "dialog-gitflow-hotfix",
        enabled,
        widgets,
    );
    changed |= text_row(
        ui,
        "Support branches",
        &mut c.support,
        "support/",
        "dialog-gitflow-support",
        enabled,
        widgets,
    );
    changed |= text_row(
        ui,
        "Version tags",
        &mut c.version_tag,
        "(none) or v",
        "dialog-gitflow-version-tag",
        enabled,
        widgets,
    );
    changed
}

fn start_inputs(
    ui: &mut Ui,
    state: &mut DialogState,
    kind: GitFlowKind,
    enabled: bool,
    widgets: &mut std::collections::HashMap<String, UiControl>,
) -> bool {
    let prefix = state.config.prefix(kind).to_owned();
    let (label, hint) = if kind.versioned() {
        ("Version", "1.2.0")
    } else {
        ("Feature name", "login")
    };
    let mut changed = text_row(
        ui,
        label,
        &mut state.name,
        hint,
        "dialog-gitflow-name",
        enabled,
        widgets,
    );
    if !state.name.is_empty() {
        pages::note(ui, &format!("Branch: {prefix}{}", state.name));
    }
    if kind == GitFlowKind::Support {
        changed |= text_row(
            ui,
            "Start from (branch, tag or commit)",
            &mut state.base,
            &state.config.master.clone(),
            "dialog-gitflow-base",
            enabled,
            widgets,
        );
    }
    changed
}

fn finish_inputs(
    ui: &mut Ui,
    state: &mut DialogState,
    enabled: bool,
    widgets: &mut std::collections::HashMap<String, UiControl>,
) -> bool {
    let mut changed = false;
    pages::caption(ui, "Branch to finish");
    let branches: Vec<(String, GitFlowKind)> = state
        .finishable()
        .iter()
        .map(|b| (b.branch.clone(), b.kind))
        .collect();
    if branches.is_empty() {
        pages::note(
            ui,
            "There are no feature, release or hotfix branches. Start one first.",
        );
        return false;
    }
    for (i, (branch, kind)) in branches.iter().enumerate() {
        let selected = state.branch.as_deref() == Some(branch.as_str());
        let r = ui.add_enabled(
            enabled,
            RadioButton::new(selected, format!("{branch}  ·  {}", kind.label())),
        );
        pages::control(
            widgets,
            format!("dialog-gitflow-branch-{i}"),
            &r,
            ui.clip_rect(),
        );
        if r.clicked() && !selected {
            state.branch = Some(branch.clone());
            state.options.tag_message = default_message(state);
            changed = true;
        }
    }
    let kind = state.chosen().map(|b| b.kind);
    pages::caption(ui, "Options");
    if kind == Some(GitFlowKind::Feature) {
        let r = ui.add_enabled(
            enabled,
            checkbox(
                &mut state.options.no_ff,
                "Always create a merge commit (no fast-forward)",
            ),
        );
        pages::control(widgets, "dialog-gitflow-no-ff", &r, ui.clip_rect());
        changed |= r.changed();
    }
    let r = ui.add_enabled(
        enabled,
        checkbox(
            &mut state.options.delete_branch,
            "Delete the branch afterwards",
        ),
    );
    pages::control(widgets, "dialog-gitflow-delete", &r, ui.clip_rect());
    changed |= r.changed();
    if kind.is_some_and(GitFlowKind::tagged) {
        ui.label("Tag message");
        let r = ui.add_enabled(
            enabled,
            TextEdit::multiline(&mut state.options.tag_message)
                .desired_rows(2)
                .char_limit(GITFLOW_MESSAGE_LIMIT)
                .desired_width(f32::INFINITY),
        );
        pages::control(widgets, "dialog-gitflow-message", &r, ui.clip_rect());
        changed |= r.changed();
    }
    changed
}

fn init_review(ui: &mut Ui, p: &GitFlowInitPreview) {
    pages::caption(ui, "What will happen");
    pages::well(ui, |ui| {
        if p.changes().is_empty() {
            pages::field(ui, "Settings", "Already set exactly like this", false);
        }
        for (key, value) in p.changes() {
            let value = if value.is_empty() { "(empty)" } else { value };
            pages::field(ui, "Set", &format!("{key} = {value}"), true);
        }
        let develop = &p.config().develop;
        let line = match p.develop() {
            GitFlowDevelop::Exists(id) => format!("{develop} exists at {}", short(id)),
            GitFlowDevelop::Create {
                source,
                target,
                track: false,
            } => format!("Create {develop} at {} from {source}", short(target)),
            GitFlowDevelop::Create { source, target, .. } => format!(
                "Create {develop} at {} from {source}, tracking it",
                short(target)
            ),
        };
        pages::field(ui, "Development", &line, false);
        pages::field(
            ui,
            "Production",
            &format!("{} at {}", p.config().master, short(p.production())),
            false,
        );
    });
    if p.unchanged() {
        pages::callout(
            ui,
            BLUE(),
            "Git Flow is already set up with these values. Nothing would change.",
        );
    }
}

fn start_review(ui: &mut Ui, p: &GitFlowStartPreview) {
    pages::caption(ui, "What will happen");
    pages::well(ui, |ui| {
        pages::field(ui, "Create", p.branch(), true);
        pages::field(
            ui,
            "Starts at",
            &format!("{} ({})", p.base(), short(p.base_id())),
            false,
        );
        if let Some(tag) = p.tag() {
            pages::field(
                ui,
                "Version tag",
                &format!("{tag} (made when finished)"),
                false,
            );
        }
        pages::field(ui, "Then", &format!("Switch to {}", p.branch()), false);
    });
    if p.carried() > 0 {
        pages::callout(
            ui,
            AMBER(),
            &format!(
                "{} changed tracked file{} come along to the new branch. Git refuses the switch if they would be overwritten.",
                p.carried(),
                if p.carried() == 1 { "" } else { "s" }
            ),
        );
    }
    if let Some(n) = p.unpushed().filter(|n| *n > 0) {
        pages::callout(
            ui,
            BLUE(),
            &format!(
                "{} has {n} commit{} that origin/{} lacks. Nothing is pushed.",
                p.base(),
                if n == 1 { "" } else { "s" },
                p.base()
            ),
        );
    }
    pages::note(ui, "Nothing is pushed.");
}

fn finish_review(ui: &mut Ui, p: &GitFlowFinishPreview) {
    pages::caption(ui, "What will happen");
    let tags: Vec<&str> = p
        .steps()
        .iter()
        .filter_map(|s| match s {
            GitFlowStep::Tag { name, .. } => Some(name.as_str()),
            _ => None,
        })
        .collect();
    pages::well(ui, |ui| {
        for (i, step) in p.steps().iter().enumerate() {
            let line = match step {
                GitFlowStep::Switch { branch } => format!("Switch to {branch}."),
                GitFlowStep::Merge {
                    branch,
                    tip,
                    into,
                    mode,
                    fast_forward,
                    message,
                } => match (mode, fast_forward) {
                    (MergeMode::AllowFastForward, true) => format!(
                        "Fast-forward {into} to {branch} ({}); no merge commit (--ff).",
                        short(tip)
                    ),
                    (MergeMode::AllowFastForward, false) => format!(
                        "Merge {branch} ({}) into {into} (--ff): a merge commit \"{message}\", as {into} has moved on.",
                        short(tip)
                    ),
                    _ => format!(
                        "Merge {branch} ({}) into {into} with a merge commit (--no-ff): \"{message}\".",
                        short(tip)
                    ),
                },
                GitFlowStep::Tag { name, message } => format!(
                    "Create annotated tag {name} on {}'s new commit: \"{}\".",
                    p.config().master,
                    message.lines().next().unwrap_or_default()
                ),
                GitFlowStep::Delete { branch } => {
                    format!("Delete {branch} with Git's safe delete (git branch -d).")
                }
            };
            ui.add(Label::new(RichText::new(format!("{}. {line}", i + 1)).size(12.5)).wrap());
        }
    });
    for note in p.notes() {
        let text = match note {
            GitFlowNote::Merged { into } => format!(
                "{} is already merged into {into}; that merge is skipped.",
                p.branch()
            ),
            GitFlowNote::Tagged { name, target } => format!(
                "Tag {name} already marks {}; tagging is skipped.",
                short(target)
            ),
            GitFlowNote::Unpushed { branch, commits } => format!(
                "{branch} has {commits} commit{} that origin/{branch} lacks.",
                if *commits == 1 { "" } else { "s" }
            ),
            GitFlowNote::ReleaseBranch { branch } => format!(
                "{branch} exists. Classic git-flow merges this hotfix into {} only; merge it into {branch} yourself if that release needs it.",
                p.config().develop
            ),
        };
        pages::callout(ui, BLUE(), &text);
    }
    match p.keep() {
        Some(GitFlowKeep::Requested) => pages::note(ui, &format!("{} is kept.", p.branch())),
        Some(GitFlowKeep::CheckedOut(path)) => pages::callout(
            ui,
            AMBER(),
            &format!(
                "{} is kept: it is checked out in {}, where Git refuses to delete it.",
                p.branch(),
                crate::views::display_path(path)
            ),
        ),
        Some(GitFlowKeep::Unpushed(upstream)) => pages::callout(
            ui,
            AMBER(),
            &format!(
                "{} is kept: {upstream} lacks some of its commits, so Git's safe delete would refuse it. Push it or delete it later.",
                p.branch()
            ),
        ),
        None => {}
    }
    let mut pushed = vec![p.config().develop.clone()];
    if p.kind().tagged() {
        pushed.insert(0, p.config().master.clone());
    }
    pushed.extend(tags.iter().map(|t| format!("tag {t}")));
    pages::callout(
        ui,
        BLUE(),
        &format!(
            "Ends on {}. Nothing is pushed: push {} yourself when ready.",
            p.ends_on().unwrap_or("the current branch"),
            pushed.join(", ")
        ),
    );
}

/// Activity text for Initialize.
pub(crate) fn init_outcome(out: &mut Outcome, report: GitFlowInitReport) {
    let mut ok = true;
    if let Some(branch) = report.branch {
        ok &= branch.write.result.is_ok();
        out.branch(branch);
    }
    if let Some(config) = report.config {
        ok &= out.observe("Write Git Flow settings", config).is_some();
    }
    let state = out.observe("Read Git Flow settings", report.state);
    out.command_succeeded = ok && state.as_ref().is_some_and(|s| s.config.is_some());
    out.summary = if out.command_succeeded {
        "Git Flow initialized in this repository's config; nothing switched or pushed.".into()
    } else {
        "Git Flow was not fully initialized. Review the details, then Initialize again.".into()
    };
}

/// Activity text for Start.
pub(crate) fn start_outcome(
    out: &mut Outcome,
    preview: &GitFlowStartPreview,
    report: GitFlowStartReport,
) {
    let created = report.create.write.result.is_ok();
    out.branch(report.create);
    let switched = match report.switch {
        Some(Ok(switch)) => {
            let ok = switch.write.result.is_ok();
            out.branch(switch);
            ok
        }
        Some(Err(error)) => {
            out.observe::<()>("Switch branch", Err(error));
            false
        }
        None => false,
    };
    out.command_succeeded = created && switched;
    out.summary = match (created, switched) {
        (true, true) => format!(
            "Started {} from {}; nothing pushed.",
            preview.branch(),
            preview.base()
        ),
        (true, false) => format!(
            "Created {}, but did not switch to it. Review the details before retrying.",
            preview.branch()
        ),
        _ => format!(
            "{} was not created. Review the details before retrying.",
            preview.branch()
        ),
    };
}

/// Activity text for Finish: each attempted step through the existing branch, merge
/// and tag reports, then what was skipped.
pub(crate) fn finish_outcome(
    out: &mut Outcome,
    preview: &GitFlowFinishPreview,
    report: GitFlowFinishReport,
) {
    let total = preview.steps().len();
    let mut done = 0;
    let mut conflicts = false;
    for (step, result) in report.steps {
        let mut part = Outcome::default();
        let succeeded = match result {
            Ok(GitFlowStepReport::Branch(r)) => {
                let ok = r.write.result.is_ok();
                part.branch(r);
                ok
            }
            Ok(GitFlowStepReport::Merge(r)) => {
                let ok = r.write.result.is_ok() && r.failure.is_none();
                conflicts |= r.failure == Some(MergeFailure::Conflicts);
                part.merge(r);
                ok
            }
            Ok(GitFlowStepReport::Tag(r)) => {
                let ok = r.result.is_ok() && r.verification.is_ok() && r.failure.is_none();
                part.tag(r);
                ok
            }
            Err(error) => {
                part.observe::<()>("Step", Err(error));
                false
            }
        };
        let label = step_label(&step);
        if succeeded {
            done += 1;
            out.details.push(format!("Done: {label}"));
        } else {
            out.details.push(format!("Stopped: {label}"));
            out.details.extend(part.details);
        }
        out.raw_output.push_str(&part.raw_output);
    }
    if !report.skipped.is_empty() {
        out.details.push(format!(
            "Not run: {}",
            report
                .skipped
                .iter()
                .map(step_label)
                .collect::<Vec<_>>()
                .join("; ")
        ));
    }
    let state = out.observe("Refresh operation", report.state);
    out.observe("Refresh branches", report.branches);
    if let Some(failure) = report.failure {
        out.details
            .push(format!("[{}] {}", failure.code(), failure.guidance()));
    }
    out.command_succeeded = done == total && report.failure.is_none() && !report.cancelled;
    out.recovery_pending = conflicts || state.is_some_and(|s| !s.is_idle());
    out.summary = if out.command_succeeded {
        format!("Finished {}; nothing pushed.", preview.branch())
    } else if conflicts {
        format!(
            "Finishing {} stopped at a merge with conflicts. Resolve them in Changes and Continue merge, then Finish again.",
            preview.branch()
        )
    } else if report.cancelled {
        format!(
            "Finishing {} was cancelled after {done} of {total} steps; cancellation is not rollback.",
            preview.branch()
        )
    } else {
        format!(
            "Finishing {} stopped after {done} of {total} steps. Review the details, then Finish again to continue.",
            preview.branch()
        )
    };
}

fn step_label(step: &GitFlowStep) -> String {
    match step {
        GitFlowStep::Switch { branch } => format!("switch to {branch}"),
        GitFlowStep::Merge { branch, into, .. } => format!("merge {branch} into {into}"),
        GitFlowStep::Tag { name, .. } => format!("create tag {name}"),
        GitFlowStep::Delete { branch } => format!("delete {branch}"),
    }
}

/// Initialize, Start and Finish through the dialogs' own buttons on a real repository.
#[cfg(all(test, windows))]
mod flow_tests {
    use super::Request;
    use crate::{
        app::{DialogKind, Intent},
        batch::flow_tests::{click, open, wait},
        test_common as common,
    };
    use eframe::egui::Context;
    use rocket_git::*;

    fn text(f: &common::Fixture, arguments: &[&str]) -> String {
        String::from_utf8(f.run(arguments))
            .unwrap()
            .trim_end()
            .to_owned()
    }
    fn review_and_submit(ctx: &Context, app: &mut crate::app::Rocket) {
        click(ctx, app, "dialog-preview-refresh");
        wait(ctx, app);
        let state = app.dialog.as_ref().unwrap().gitflow.as_ref().unwrap();
        assert!(
            state.preview.is_some(),
            "{:?}",
            app.dialog.as_ref().unwrap().error
        );
        click(ctx, app, "dialog-submit");
        wait(ctx, app);
        assert!(
            app.dialog.is_none(),
            "the dialog closes after a successful write"
        );
        let activity = app.activities.last().unwrap();
        assert!(activity.outcome.command_succeeded, "{:?}", activity.outcome);
    }

    #[test]
    fn initialize_start_and_finish_through_reviewed_dialogs() {
        common::isolated(
            "ROCKET_GITFLOW_UI_CASE",
            "gitflow::flow_tests::initialize_start_and_finish_through_reviewed_dialogs",
            || {
                let f = common::Fixture::new();
                f.write("app.txt", "base\n");
                f.commit("base");
                let ctx = Context::default();
                let mut app = open(&ctx, &f);
                // Start before Initialize explains instead of offering a review.
                app.dispatch(Intent::Dialog(DialogKind::GitFlow(Request::Start(
                    GitFlowKind::Feature,
                ))));
                wait(&ctx, &mut app);
                assert!(!crate::batch::flow_tests::enabled(
                    &ctx,
                    &mut app,
                    "dialog-preview-refresh"
                ));
                click(&ctx, &mut app, "dialog-cancel");

                app.dispatch(Intent::Dialog(DialogKind::GitFlow(Request::Init)));
                wait(&ctx, &mut app);
                let state = app.dialog.as_ref().unwrap().gitflow.as_ref().unwrap();
                assert_eq!(state.config, GitFlowConfig::defaults("main"));
                review_and_submit(&ctx, &mut app);
                assert_eq!(text(&f, &["config", "gitflow.branch.develop"]), "develop");
                assert_eq!(text(&f, &["symbolic-ref", "--short", "HEAD"]), "main");

                app.dispatch(Intent::Dialog(DialogKind::GitFlow(Request::Start(
                    GitFlowKind::Feature,
                ))));
                wait(&ctx, &mut app);
                app.dialog.as_mut().unwrap().gitflow.as_mut().unwrap().name = "login".into();
                review_and_submit(&ctx, &mut app);
                assert_eq!(
                    text(&f, &["symbolic-ref", "--short", "HEAD"]),
                    "feature/login"
                );

                f.write("login.txt", "login\n");
                f.commit("login");
                let tip = text(&f, &["rev-parse", "HEAD"]);
                app.dispatch(Intent::Dialog(DialogKind::GitFlow(Request::Finish)));
                wait(&ctx, &mut app);
                let state = app.dialog.as_ref().unwrap().gitflow.as_ref().unwrap();
                assert_eq!(state.branch.as_deref(), Some("feature/login"));
                review_and_submit(&ctx, &mut app);
                assert_eq!(text(&f, &["rev-parse", "develop"]), tip);
                assert_eq!(text(&f, &["symbolic-ref", "--short", "HEAD"]), "develop");
                assert!(text(&f, &["branch", "--list", "feature/login"]).is_empty());
                let summary = &app.activities.last().unwrap().outcome.summary;
                assert_eq!(summary, "Finished feature/login; nothing pushed.");
            },
        );
    }
}
