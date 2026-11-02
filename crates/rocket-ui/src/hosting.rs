//! GitHub page: this repository's GitHub links, and the signed-in account's
//! repositories and notifications. Read-only: GitHub is never changed, and links
//! open validated github.com pages in the browser. Requests run on background
//! tasks; the sign-in is kept in memory for the rest of this run.
use crate::{
    app::*,
    backend::user_error,
    pages::{self, control},
    style::*,
};
use eframe::egui::*;
use rocket_git::*;
use std::{collections::HashMap, sync::Arc};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Section {
    #[default]
    Repository,
    Repositories,
    Notifications,
}
impl Section {
    const ALL: [Self; 3] = [Self::Repository, Self::Repositories, Self::Notifications];
}

/// GitHub's view of the open repository and its checked-out branch.
#[derive(Clone, Debug)]
pub struct RepositoryLinks {
    pub remote: String,
    pub repository: GitHubRepository,
    pub branch: Option<String>,
    /// The checked-out branch's upstream branch on that remote.
    pub pushed: Option<String>,
    pub ahead: u64,
    /// Where a pull request would go: the fork's parent, or the repository itself.
    pub base: GitHubRemote,
    pub base_branch: String,
    pub pull: Option<GitHubPullRequest>,
}

/// What the tab's snapshot says about GitHub, before any request.
struct Local {
    remote: String,
    github: GitHubRemote,
    branch: Option<String>,
    pushed: Option<String>,
    ahead: u64,
}

enum Data {
    Links(Option<Box<RepositoryLinks>>),
    Repositories(GitHubList<GitHubRepository>, u32),
    Notifications(GitHubList<GitHubNotification>, u32),
}
struct Loaded {
    session: Arc<GitHubSession>,
    data: Data,
}

#[derive(Default)]
pub struct DialogState {
    pub section: Section,
    pub links: Option<RepositoryLinks>,
    /// The repository has no github.com remote.
    pub no_remote: bool,
    pub repositories: Vec<GitHubRepository>,
    pub repositories_next: Option<u32>,
    pub notifications: Vec<GitHubNotification>,
    pub notifications_next: Option<u32>,
    pub all_notifications: bool,
    pub filter: String,
    /// Index into the current section's full list.
    pub selected: Option<usize>,
    /// Sections read at least once, by `Section` order.
    pub loaded: [bool; 3],
    /// No usable sign-in: offer Git Credential Manager's sign-in.
    pub sign_in: bool,
    pub notice: Option<String>,
    task: Option<ReadTask<Loaded>>,
}
impl DialogState {
    pub fn running(&self) -> bool {
        self.task.is_some()
    }
    fn matches(&self, text: &str) -> bool {
        let filter = self.filter.trim().to_lowercase();
        filter.is_empty() || text.to_lowercase().contains(&filter)
    }
    pub fn visible_repositories(&self) -> Vec<usize> {
        (0..self.repositories.len())
            .filter(|&i| {
                let r = &self.repositories[i];
                self.matches(&r.remote.full_name())
                    || r.description.as_deref().is_some_and(|d| self.matches(d))
            })
            .collect()
    }
    pub fn visible_notifications(&self) -> Vec<usize> {
        (0..self.notifications.len())
            .filter(|&i| {
                let n = &self.notifications[i];
                self.matches(&n.title) || self.matches(&n.repository.full_name())
            })
            .collect()
    }
}

fn text(bytes: &[u8]) -> Option<String> {
    String::from_utf8(bytes.to_vec()).ok()
}

/// The GitHub remote this tab's branch uses: its upstream's remote, else origin,
/// else the first github.com remote. `None` while the repository is still loading.
fn local_github(tab: &Tab) -> Option<Option<Local>> {
    let snapshot = tab.snapshot.as_ref()?;
    let on_github = |remote: &Remote| {
        let github = remote.urls.first().and_then(|u| GitHubRemote::parse(u))?;
        Some((text(&remote.name)?, github))
    };
    let branch = match &snapshot.head {
        Some(Head::Branch { name, .. }) => text(name),
        _ => None,
    };
    // `refs/remotes/<remote>/<branch>`; remote names may themselves contain '/'.
    let tracked = snapshot
        .branches
        .iter()
        .find(|b| b.is_head)
        .and_then(|b| b.upstream.as_deref())
        .and_then(text)
        .and_then(|upstream| {
            let short = upstream.strip_prefix("refs/remotes/")?.to_owned();
            snapshot
                .remotes
                .iter()
                .filter_map(|r| {
                    let name = text(&r.name)?;
                    let pushed = short.strip_prefix(&format!("{name}/"))?.to_owned();
                    Some((name.len(), pushed, r))
                })
                .max_by_key(|(length, ..)| *length)
        });
    if let Some((_, pushed, remote)) = tracked
        && let Some((remote, github)) = on_github(remote)
    {
        let ahead = snapshot
            .status
            .as_ref()
            .and_then(|s| s.ahead_behind)
            .map_or(0, |(ahead, _)| ahead);
        return Some(Some(Local {
            remote,
            github,
            branch,
            pushed: Some(pushed),
            ahead,
        }));
    }
    let found = snapshot
        .remotes
        .iter()
        .find(|r| r.name == b"origin")
        .and_then(on_github)
        .or_else(|| snapshot.remotes.iter().find_map(on_github));
    Some(found.map(|(remote, github)| Local {
        remote,
        github,
        branch,
        pushed: None,
        ahead: 0,
    }))
}

/// GitHub's notification types and reasons, in words.
fn kind_label(kind: &str) -> &str {
    match kind {
        "PullRequest" => "Pull request",
        "CheckSuite" => "Check suite",
        "WorkflowRun" => "Workflow run",
        "RepositoryVulnerabilityAlert" | "RepositoryDependabotAlertsThread" => "Security alert",
        "RepositoryInvitation" => "Invitation",
        other => other,
    }
}
fn reason_label(reason: &str) -> String {
    reason.replace('_', " ")
}
/// `2026-10-21T12:34:56Z` as `2026-10-21 12:34 UTC`; anything else unchanged.
fn when(time: &str) -> String {
    match (time.get(..10), time.get(11..16), time.as_bytes().get(10)) {
        (Some(day), Some(minute), Some(b'T')) if time.ends_with('Z') => {
            format!("{day} {minute} UTC")
        }
        _ => time.to_owned(),
    }
}

/// Explorer hands an https URL to the default browser. The URL passed
/// [`github_web_url`] or `rocket_git::https_url`: one argument without spaces
/// or quotes.
pub(crate) fn launch_browser(url: &str) -> Result<()> {
    rocket_git::ensure_unprivileged_process()?;
    let explorer = std::env::var_os("SystemRoot")
        .map(|root| std::path::PathBuf::from(root).join("explorer.exe"))
        .ok_or_else(|| Error::Io {
            action: "open the browser",
            source: std::io::Error::new(std::io::ErrorKind::NotFound, "SystemRoot is not set"),
        })?;
    std::process::Command::new(explorer)
        .arg(url)
        .spawn()
        .map(drop)
        .map_err(|source| Error::Io {
            action: "open the browser",
            source,
        })
}

#[derive(Default)]
struct Flags {
    dismiss: bool,
    refresh: bool,
    sign_in: bool,
    cancel: bool,
    more: bool,
    section: Option<Section>,
    open: Option<String>,
    clone: Option<String>,
}

impl Rocket {
    /// Read what the GitHub page's current section shows. Without a session this
    /// signs in first; Git Credential Manager may prompt only when `interactive`.
    pub(crate) fn start_github_read(&mut self, interactive: bool, more: bool) {
        let Some(d) = &mut self.dialog else { return };
        if !matches!(d.kind, DialogKind::GitHub) {
            return;
        }
        let s = d.github.get_or_insert_with(Default::default);
        // Replacing a read drops it, which cancels its commands.
        s.task = None;
        s.notice = None;
        d.error = None;
        let Some(endpoint) = self.github_endpoint else {
            d.error = Some(user_error(&Error::Hosting(HostingFailure::Disabled)));
            return;
        };
        let Some(backend) = self.backend.clone() else {
            return;
        };
        let section = s.section;
        let mut local = None;
        if section == Section::Repository {
            let tab = self.tabs.iter().find(|t| Some(t.id) == d.owner);
            match tab.and_then(local_github) {
                None => {
                    s.notice = Some("The repository is still loading. Refresh in a moment.".into());
                    return;
                }
                Some(None) => {
                    s.links = None;
                    s.no_remote = true;
                    s.loaded[0] = true;
                    // Nothing to read here; sign in only when asked to.
                    if self.github.is_some() || !interactive {
                        return;
                    }
                }
                Some(found) => local = found,
            }
        }
        let page = match section {
            Section::Repositories if more => s.repositories_next.unwrap_or(1),
            Section::Notifications if more => s.notifications_next.unwrap_or(1),
            _ => 1,
        };
        let all = s.all_notifications;
        let session = self.github.clone();
        s.notice = Some(if session.is_none() {
            "Signing in with Git's stored github.com credential…".into()
        } else {
            "Reading GitHub…".into()
        });
        match ReadTask::spawn(move |cancel| {
            let session = match session {
                Some(session) => session,
                None => Arc::new(backend.github_session(endpoint, interactive, &cancel)?),
            };
            let data = match section {
                Section::Repository => Data::Links(match local {
                    None => None,
                    Some(local) => {
                        let repository = backend.github("github-repository", &session, |g| {
                            g.repository(&local.github, &cancel)
                        })?;
                        let (base, base_branch) = repository.parent.clone().unwrap_or_else(|| {
                            (repository.remote.clone(), repository.default_branch.clone())
                        });
                        let pull = match &local.pushed {
                            Some(branch) => backend.github("github-pulls", &session, |g| {
                                g.open_pull_request(
                                    &base,
                                    &repository.remote.owner,
                                    branch,
                                    &cancel,
                                )
                            })?,
                            None => None,
                        };
                        Some(Box::new(RepositoryLinks {
                            remote: local.remote,
                            repository,
                            branch: local.branch,
                            pushed: local.pushed,
                            ahead: local.ahead,
                            base,
                            base_branch,
                            pull,
                        }))
                    }
                }),
                Section::Repositories => Data::Repositories(
                    backend.github("github-repositories", &session, |g| {
                        g.repositories(page, &cancel)
                    })?,
                    page,
                ),
                Section::Notifications => Data::Notifications(
                    backend.github("github-notifications", &session, |g| {
                        g.notifications(all, page, &cancel)
                    })?,
                    page,
                ),
            };
            Ok(Loaded { session, data })
        }) {
            Ok(task) => s.task = Some(task),
            Err(error) => {
                s.notice = None;
                d.error = Some(user_error(&error));
            }
        }
    }

    pub(crate) fn poll_github(&mut self) {
        let Some(d) = &mut self.dialog else { return };
        let Some(s) = &mut d.github else { return };
        let Some(result) = s.task.as_mut().and_then(ReadTask::try_finish) else {
            return;
        };
        s.task = None;
        s.notice = None;
        match result {
            Ok(Loaded { session, data }) => {
                self.github = Some(session);
                s.sign_in = false;
                match data {
                    Data::Links(links) => {
                        s.no_remote = links.is_none();
                        s.links = links.map(|links| *links);
                        s.loaded[0] = true;
                    }
                    // A first page replaces the list. Later pages skip entries
                    // that moved there since (repositories re-sort as they are pushed).
                    Data::Repositories(list, page) => {
                        if page <= 1 {
                            s.repositories.clear();
                            if s.section == Section::Repositories {
                                s.selected = None;
                            }
                        }
                        for item in list.items {
                            if !s.repositories.iter().any(|r| r.remote == item.remote) {
                                s.repositories.push(item);
                            }
                        }
                        s.repositories_next = list.next_page;
                        s.loaded[1] = true;
                    }
                    Data::Notifications(list, page) => {
                        if page <= 1 {
                            s.notifications.clear();
                            if s.section == Section::Notifications {
                                s.selected = None;
                            }
                        }
                        for item in list.items {
                            if !s.notifications.iter().any(|n| n.id == item.id) {
                                s.notifications.push(item);
                            }
                        }
                        s.notifications_next = list.next_page;
                        s.loaded[2] = true;
                    }
                }
            }
            Err(Error::Cancelled) => {
                s.notice = Some("GitHub read cancelled. Earlier results are kept.".into());
            }
            Err(error) => {
                if matches!(
                    error,
                    Error::Hosting(HostingFailure::Unauthorized | HostingFailure::SignInRequired)
                ) {
                    // GitHub refused the session's token; the next read signs in again.
                    self.github = None;
                    s.sign_in = true;
                }
                d.error = Some(user_error(&error));
            }
        }
    }

    /// The GitHub page has no review dialog. This only drops a page whose
    /// repository is no longer active; dropping its read cancels it.
    pub(crate) fn github_dialog(&mut self) {
        let active = self.tab().map(|t| t.id);
        if self.dialog.as_ref().is_some_and(|d| d.owner != active) {
            self.dialog = None;
        }
    }

    /// Open a github.com page in the default browser. The MCP test host records
    /// it instead.
    pub(crate) fn open_github(&mut self, url: &str) {
        let Some(url) = github_web_url(url) else {
            if let Some(d) = &mut self.dialog {
                d.error = Some("Rocket opens only github.com pages.".into());
            }
            return;
        };
        let opened = &mut self.opened_urls;
        let mut open = || match opened {
            Some(list) => {
                list.push(url.to_owned());
                Ok(())
            }
            None => launch_browser(url),
        };
        let result = match &self.backend {
            Some(backend) => backend.log.request("github-open", |_| open()),
            None => open(),
        };
        if let (Err(error), Some(d)) = (result, &mut self.dialog) {
            d.error = Some(user_error(&error));
        }
    }

    pub(crate) fn github_page(&mut self, ui: &mut Ui) {
        let can_clone = self.backend.is_some() && self.write.is_none();
        let login = self.github.as_ref().map(|g| g.account().login.clone());
        let Some(mut d) = self.dialog.take() else {
            return;
        };
        let s = d.github.get_or_insert_with(Default::default);
        let waiting = s.running();
        let mut f = Flags::default();
        let widgets = &mut self.frame_widgets;
        let description = match &login {
            Some(login) => {
                format!("Signed in as {login} through Git's credential helper · read-only")
            }
            None => "Uses the github.com sign-in Git already stores · read-only".to_owned(),
        };
        pages::header(ui, Icon::Remote, "GitHub", &description, |ui| {
            let r = pages::ghost(ui, true, "Close");
            control(widgets, "dialog-cancel", &r, ui.clip_rect());
            f.dismiss = r.clicked();
            if waiting {
                let r = ui.button("Stop");
                control(widgets, "github-cancel", &r, ui.clip_rect());
                f.cancel = r.clicked();
                ui.spinner();
            } else {
                let r = ui.button("Refresh");
                control(widgets, "github-refresh", &r, ui.clip_rect());
                f.refresh = r.clicked();
                if s.sign_in {
                    let r = pages::primary(ui, true, "Sign in…");
                    control(widgets, "github-sign-in", &r, ui.clip_rect());
                    f.sign_in = r.clicked();
                }
            }
        });
        let unread = s.notifications.iter().filter(|n| n.unread).count();
        let tab = pages::tabs(
            ui,
            &[
                pages::TabItem {
                    label: "This repository",
                    count: None,
                    selected: s.section == Section::Repository,
                    enabled: true,
                    key: "github-tab-repository",
                },
                pages::TabItem {
                    label: "Repositories",
                    count: s.loaded[1].then_some(s.repositories.len()),
                    selected: s.section == Section::Repositories,
                    enabled: true,
                    key: "github-tab-repositories",
                },
                pages::TabItem {
                    label: "Notifications",
                    count: s.loaded[2].then_some(unread),
                    selected: s.section == Section::Notifications,
                    enabled: true,
                    key: "github-tab-notifications",
                },
            ],
            widgets,
        );
        f.section = tab.map(|i| Section::ALL[i]);
        if let Some(error) = &d.error {
            pages::toolbar(ui, |ui| pages::callout(ui, RED(), error));
        } else if let Some(notice) = &s.notice {
            pages::toolbar(ui, |ui| pages::note(ui, notice));
        }
        match s.section {
            Section::Repository => repository_section(ui, s, waiting, widgets, &mut f),
            Section::Repositories => {
                repositories_section(ui, s, waiting, can_clone, widgets, &mut f)
            }
            Section::Notifications => notifications_section(ui, s, waiting, widgets, &mut f),
        }
        pages::details_scroll(ui, widgets, true);
        self.dialog = Some(d);
        self.finish_github_frame(f);
    }

    fn finish_github_frame(&mut self, f: Flags) {
        if f.dismiss {
            self.dialog = None;
            return;
        }
        if let Some(url) = f.open {
            self.open_github(&url);
        }
        if let Some(url) = f.clone {
            // Clone replaces this page; the source stays editable there.
            self.show_dialog(DialogKind::Clone);
            if let Some(d) = &mut self.dialog {
                d.first = url;
            }
            return;
        }
        let Some(s) = self.dialog.as_mut().and_then(|d| d.github.as_mut()) else {
            return;
        };
        if f.cancel {
            if let Some(task) = &s.task {
                task.cancel();
            }
        } else if let Some(section) = f.section {
            if section != s.section {
                s.section = section;
                s.selected = None;
                s.filter.clear();
                if !s.loaded[section as usize] {
                    self.start_github_read(false, false);
                }
            }
        } else if f.sign_in {
            self.start_github_read(true, false);
        } else if f.refresh {
            self.start_github_read(false, false);
        } else if f.more {
            self.start_github_read(false, true);
        }
    }
}

fn button(
    ui: &mut Ui,
    widgets: &mut HashMap<String, UiControl>,
    key: &str,
    label: &str,
    enabled: bool,
) -> bool {
    let r = ui.add_enabled(enabled, Button::new(label));
    control(widgets, key, &r, ui.clip_rect());
    r.clicked()
}

fn repository_section(
    ui: &mut Ui,
    s: &DialogState,
    waiting: bool,
    widgets: &mut HashMap<String, UiControl>,
    f: &mut Flags,
) {
    let output = ScrollArea::vertical()
        .id_salt("github-repository")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            Frame::new()
                .inner_margin(Margin {
                    left: 24,
                    right: 24,
                    top: 8,
                    bottom: 12,
                })
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.spacing_mut().item_spacing.y = 6.0;
                    repository_links(ui, s, waiting, widgets, f);
                });
        });
    pages::region(
        widgets,
        "github-scroll",
        output.inner_rect.intersect(ui.clip_rect()),
        true,
    );
}

fn repository_links(
    ui: &mut Ui,
    s: &DialogState,
    waiting: bool,
    widgets: &mut HashMap<String, UiControl>,
    f: &mut Flags,
) {
    let Some(links) = &s.links else {
        if s.no_remote {
            pages::empty(
                ui,
                "This repository has no github.com remote. Add one under Manage Remotes to see its GitHub links here.",
            );
        } else if waiting {
            pages::loading(ui, "Reading GitHub…");
        } else {
            pages::empty(ui, "Refresh to read this repository's GitHub links.");
        }
        return;
    };
    let repository = &links.repository;
    pages::well(ui, |ui| {
        let mut kind = vec![if repository.private {
            "private"
        } else {
            "public"
        }];
        if repository.fork {
            kind.push("fork");
        }
        if repository.archived {
            kind.push("archived");
        }
        pages::field(
            ui,
            "Repository",
            &format!("{} · {}", repository.remote.full_name(), kind.join(" · ")),
            false,
        );
        pages::field(ui, "Remote", &links.remote, false);
        pages::field(
            ui,
            "Branch",
            links.branch.as_deref().unwrap_or("Detached HEAD"),
            false,
        );
        let pushed = match &links.pushed {
            Some(pushed) if links.ahead > 0 => format!(
                "{pushed} · {} local commit{} not pushed",
                links.ahead,
                if links.ahead == 1 { "" } else { "s" }
            ),
            Some(pushed) => pushed.clone(),
            None => "Not pushed (no upstream branch on this remote)".into(),
        };
        pages::field(ui, "On GitHub", &pushed, false);
        pages::field(
            ui,
            "Pull requests to",
            &format!("{} · {}", links.base.full_name(), links.base_branch),
            false,
        );
        let pull = links.pull.as_ref().map_or_else(
            || "None open for this branch".into(),
            |p| {
                format!(
                    "#{} {}{}",
                    p.number,
                    p.title,
                    if p.draft { " (draft)" } else { "" }
                )
            },
        );
        pages::field(ui, "Pull request", &pull, false);
    });
    ui.add_space(4.0);
    let base_branch = links.base == repository.remote
        && links.pushed.as_deref() == Some(links.base_branch.as_str());
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(8.0, 6.0);
        if let Some(pull) = &links.pull {
            let r = pages::primary(ui, true, &format!("View pull request #{}", pull.number));
            control(widgets, "github-open-pull", &r, ui.clip_rect());
            if r.clicked() {
                f.open = Some(pull.html_url.clone());
            }
        } else {
            let enabled = links.pushed.is_some() && !base_branch && !waiting;
            let r = pages::primary(ui, enabled, "Create pull request…");
            control(widgets, "github-create-pull", &r, ui.clip_rect());
            if r.clicked()
                && let Some(pushed) = &links.pushed
            {
                f.open = Some(links.base.compare_url(
                    &links.base_branch,
                    Some(repository.remote.owner.as_str()),
                    pushed,
                ));
            }
        }
        if button(
            ui,
            widgets,
            "github-open-repository",
            "View repository",
            true,
        ) {
            f.open = Some(repository.html_url.clone());
        }
        if let Some(pushed) = &links.pushed
            && button(ui, widgets, "github-open-branch", "View branch", true)
        {
            f.open = Some(repository.remote.branch_url(pushed));
        }
        if button(ui, widgets, "github-open-pulls", "Pull requests", true) {
            f.open = Some(links.base.pulls_url());
        }
    });
    if links.pushed.is_none() {
        pages::note(
            ui,
            &format!(
                "GitHub compares pushed branches only. Push this branch to {} with an upstream, then refresh.",
                links.remote
            ),
        );
    } else if base_branch {
        pages::note(
            ui,
            "This branch is the pull request base. Check out a feature branch to open a pull request.",
        );
    } else if links.pull.is_none() {
        let unpushed = if links.ahead > 0 {
            " Local commits that are not pushed are not included."
        } else {
            ""
        };
        pages::note(
            ui,
            &format!(
                "Create pull request opens GitHub's comparison page, where you write and submit it.{unpushed}"
            ),
        );
    }
}

/// Filter box and "n of m shown"; returns whether Load more was clicked.
fn list_toolbar(
    ui: &mut Ui,
    filter: &mut String,
    counts: (usize, usize),
    more: Option<bool>,
    widgets: &mut HashMap<String, UiControl>,
) -> bool {
    let r = ui.add(
        TextEdit::singleline(filter)
            .hint_text("Filter")
            .desired_width(220.0),
    );
    control(widgets, "github-filter", &r, ui.clip_rect());
    let (shown, total) = counts;
    ui.label(
        RichText::new(if shown == total {
            format!("{total} shown")
        } else {
            format!("{shown} of {total} shown")
        })
        .size(12.0)
        .color(MUTED()),
    );
    more.is_some_and(|enabled| button(ui, widgets, "github-load-more", "Load more", enabled))
}

/// One list row's text, owned for the frame.
struct Line {
    image: Icon,
    title: String,
    detail: String,
    pills: Vec<(String, Color32)>,
}

fn list<T>(
    ui: &mut Ui,
    rows: &[usize],
    items: &[T],
    selected: &mut Option<usize>,
    key: &str,
    widgets: &mut HashMap<String, UiControl>,
    line: impl Fn(&T) -> Line,
) {
    let output = ScrollArea::vertical()
        .id_salt(("github-list", key))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            for &i in rows {
                let Line {
                    image,
                    title,
                    detail,
                    pills,
                } = line(&items[i]);
                let row = pages::Row {
                    image,
                    title: &title,
                    detail: &detail,
                    pills,
                    indent: 0.0,
                };
                let r = pages::row(ui, row, *selected == Some(i), true);
                control(widgets, format!("{key}-{i}"), &r, ui.clip_rect());
                if r.clicked() {
                    *selected = Some(i);
                }
            }
        });
    pages::region(
        widgets,
        "github-list-scroll",
        output.inner_rect.intersect(ui.clip_rect()),
        true,
    );
}

fn repositories_section(
    ui: &mut Ui,
    s: &mut DialogState,
    waiting: bool,
    can_clone: bool,
    widgets: &mut HashMap<String, UiControl>,
    f: &mut Flags,
) {
    let rows = s.visible_repositories();
    let counts = (rows.len(), s.repositories.len());
    let more = s.repositories_next.map(|_| !waiting);
    pages::toolbar(ui, |ui| {
        f.more |= list_toolbar(ui, &mut s.filter, counts, more, widgets);
    });
    pages::split(ui, |ui, side| match side {
        pages::Side::List => {
            if s.repositories.is_empty() {
                if waiting {
                    pages::loading(ui, "Reading repositories…");
                } else if s.loaded[1] {
                    pages::empty(ui, "This account has no repositories.");
                }
                return;
            }
            list(
                ui,
                &rows,
                &s.repositories,
                &mut s.selected,
                "github-repository-row",
                widgets,
                |r: &GitHubRepository| {
                    let mut pills = Vec::new();
                    if r.private {
                        pills.push(("private".to_owned(), AMBER()));
                    }
                    if r.fork {
                        pills.push(("fork".to_owned(), VIOLET()));
                    }
                    if r.archived {
                        pills.push(("archived".to_owned(), FAINT()));
                    }
                    Line {
                        image: Icon::Remote,
                        title: r.remote.full_name(),
                        detail: r.description.clone().unwrap_or_default(),
                        pills,
                    }
                },
            );
        }
        pages::Side::Details => {
            let Some(r) = s.selected.and_then(|i| s.repositories.get(i)) else {
                pages::empty(ui, "Select a repository to open or clone it.");
                return;
            };
            ui.label(RichText::new(r.remote.full_name()).font(semibold(15.0)));
            if let Some(description) = &r.description {
                pages::note(ui, description);
            }
            pages::field(
                ui,
                "Visibility",
                if r.private { "Private" } else { "Public" },
                false,
            );
            pages::field(ui, "Default branch", &r.default_branch, false);
            pages::field(
                ui,
                "Last push",
                &r.pushed_at.as_deref().map_or_else(|| "Never".into(), when),
                false,
            );
            if let Some((parent, _)) = &r.parent {
                pages::field(ui, "Fork of", &parent.full_name(), false);
            }
            pages::field(ui, "HTTPS", &r.clone_url, true);
            pages::field(ui, "SSH", &r.ssh_url, true);
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                if button(
                    ui,
                    widgets,
                    "github-repository-open",
                    "Open on GitHub",
                    true,
                ) {
                    f.open = Some(r.html_url.clone());
                }
                if button(ui, widgets, "github-repository-clone", "Clone", can_clone) {
                    f.clone = Some(r.clone_url.clone());
                }
            });
        }
    });
}

fn notifications_section(
    ui: &mut Ui,
    s: &mut DialogState,
    waiting: bool,
    widgets: &mut HashMap<String, UiControl>,
    f: &mut Flags,
) {
    let rows = s.visible_notifications();
    let counts = (rows.len(), s.notifications.len());
    let more = s.notifications_next.map(|_| !waiting);
    pages::toolbar(ui, |ui| {
        f.more |= list_toolbar(ui, &mut s.filter, counts, more, widgets);
        let r = ui.add_enabled(!waiting, checkbox(&mut s.all_notifications, "Include read"));
        control(widgets, "github-notifications-all", &r, ui.clip_rect());
        f.refresh |= r.changed();
    });
    pages::split(ui, |ui, side| match side {
        pages::Side::List => {
            if s.notifications.is_empty() {
                if waiting {
                    pages::loading(ui, "Reading notifications…");
                } else if s.loaded[2] {
                    pages::empty(
                        ui,
                        if s.all_notifications {
                            "No recent notifications."
                        } else {
                            "No unread notifications."
                        },
                    );
                }
                return;
            }
            list(
                ui,
                &rows,
                &s.notifications,
                &mut s.selected,
                "github-notification-row",
                widgets,
                |n: &GitHubNotification| Line {
                    image: Icon::Activity,
                    title: n.title.clone(),
                    detail: format!(
                        "{} · {} · {}",
                        n.repository.full_name(),
                        kind_label(&n.kind),
                        reason_label(&n.reason)
                    ),
                    pills: if n.unread {
                        vec![("unread".to_owned(), BLUE())]
                    } else {
                        Vec::new()
                    },
                },
            );
        }
        pages::Side::Details => {
            let Some(n) = s.selected.and_then(|i| s.notifications.get(i)) else {
                pages::empty(ui, "Select a notification to open it on GitHub.");
                return;
            };
            ui.add(Label::new(RichText::new(&n.title).font(semibold(15.0))).wrap());
            pages::field(ui, "Repository", &n.repository.full_name(), false);
            pages::field(ui, "Type", kind_label(&n.kind), false);
            pages::field(ui, "Reason", &reason_label(&n.reason), false);
            pages::field(ui, "Updated", &when(&n.updated_at), false);
            pages::field(ui, "State", if n.unread { "Unread" } else { "Read" }, false);
            ui.add_space(6.0);
            if button(
                ui,
                widgets,
                "github-notification-open",
                "Open on GitHub",
                true,
            ) {
                f.open = Some(n.web_url.clone());
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_and_times_read_as_words() {
        assert_eq!(kind_label("PullRequest"), "Pull request");
        assert_eq!(kind_label("Issue"), "Issue");
        assert_eq!(reason_label("review_requested"), "review requested");
        assert_eq!(when("2026-10-21T12:34:56Z"), "2026-10-21 12:34 UTC");
        assert_eq!(when("yesterday"), "yesterday");
        assert_eq!(when("2026-10-21 12:34:56"), "2026-10-21 12:34:56");
    }
}

/// The page against a loopback stand-in for api.github.com and a fake credential
/// helper, driven by clicks on the rendered controls.
#[cfg(all(test, windows))]
mod flow_tests {
    use super::*;
    use crate::{app::Intent, test_common as common};
    use eframe::egui::{Context, Event, Modifiers, PointerButton, Pos2, RawInput, Rect, vec2};
    use std::{
        io::{Read, Write},
        net::TcpListener,
        path::PathBuf,
        thread,
        time::{Duration, Instant},
    };

    const CASE: &str = "ROCKET_GITHUB_UI_CASE";
    const USER: &str = r#"{"login":"rocket-fake","name":"Rocket Fake","html_url":"https://github.com/rocket-fake"}"#;

    /// One request per connection, as curl makes them.
    fn serve(route: fn(&str) -> (u16, String)) -> GitHubEndpoint {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut head = Vec::new();
                let mut byte = [0u8; 1];
                while !head.ends_with(b"\r\n\r\n") && head.len() < 65536 {
                    match stream.read(&mut byte) {
                        Ok(1) => head.push(byte[0]),
                        _ => break,
                    }
                }
                let head = String::from_utf8_lossy(&head);
                let (status, body) = route(head.split(' ').nth(1).unwrap_or(""));
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status} Fake\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
            }
        });
        GitHubEndpoint::Loopback(port)
    }
    /// A fake credential helper in this isolated test's global Git config. It
    /// records the `GCM_INTERACTIVE` value of each lookup.
    fn helper(f: &common::Fixture) -> PathBuf {
        let marker = f.root.join("helper-calls");
        let config = format!(
            "[credential]\n\thelper = \"!f() {{ cat >/dev/null; [ \\\"$1\\\" = get ] || exit 0; echo \\\"$GCM_INTERACTIVE\\\" >> '{}'; echo username=rocket-fake; echo password={}; }}; f\"\n",
            marker.to_str().unwrap().replace('\\', "/"),
            concat!("gho_", "rocketUiFake_0123")
        );
        std::fs::write(std::env::var_os("GIT_CONFIG_GLOBAL").unwrap(), config).unwrap();
        marker
    }
    fn wait(ctx: &Context, app: &mut Rocket) {
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            app.poll(ctx);
            let reading = app
                .dialog
                .as_ref()
                .and_then(|d| d.github.as_ref())
                .is_some_and(DialogState::running);
            if !app.busy() && !reading {
                return;
            }
            assert!(Instant::now() < deadline, "reads did not finish");
            thread::sleep(Duration::from_millis(10));
        }
    }
    fn frame(ctx: &Context, app: &mut Rocket, events: Vec<Event>) {
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 860.0))),
            events,
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| {
            app.frame_widgets.clear();
            if app
                .dialog
                .as_ref()
                .is_some_and(|d| matches!(d.kind, DialogKind::GitHub))
            {
                app.github_dialog();
            }
            if let Some(page) = app.page() {
                app.page_ui(ui, page);
            }
        });
        output.textures_delta.clear();
    }
    fn click(ctx: &Context, app: &mut Rocket, id: &str) {
        frame(ctx, app, Vec::new());
        frame(ctx, app, Vec::new());
        let control = app.frame_widgets.get(id).copied();
        let control = control.unwrap_or_else(|| panic!("{id} is not drawn"));
        assert!(control.enabled, "{id} is disabled");
        let pos = control.rect.center();
        let button = |pressed| Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        frame(ctx, app, vec![Event::PointerMoved(pos), button(true)]);
        frame(ctx, app, vec![button(false)]);
    }
    fn page(app: &Rocket) -> &DialogState {
        app.dialog.as_ref().unwrap().github.as_ref().unwrap()
    }

    #[test]
    fn page_links_lists_and_records_github_pages() {
        common::isolated(
            CASE,
            "hosting::flow_tests::page_links_lists_and_records_github_pages",
            || {
                let f = common::Fixture::new();
                let marker = helper(&f);
                f.write("file", "one");
                f.commit("one");
                f.run(&["switch", "-c", "feature/界"]);
                f.write("file", "two");
                f.commit("two");
                f.run(&[
                    "remote",
                    "add",
                    "origin",
                    "https://github.com/rocket-fake/forked_repo.git",
                ]);
                f.run(&["update-ref", "refs/remotes/origin/feature/界", "HEAD"]);
                f.run(&["branch", "--set-upstream-to=origin/feature/界"]);
                f.write("file", "three");
                f.commit("three");
                let endpoint = serve(|path| {
                    match path {
                    "/user" => (200, USER.into()),
                    "/repos/rocket-fake/forked_repo" => (
                        200,
                        r#"{"name":"forked_repo","owner":{"login":"rocket-fake"},"fork":true,"private":true,"default_branch":"dev","html_url":"https://github.com/rocket-fake/forked_repo","parent":{"name":"upstream","owner":{"login":"rocket-org"},"default_branch":"main"}}"#.into(),
                    ),
                    p if p.starts_with("/repos/rocket-org/upstream/pulls?")
                        && p.contains("head=rocket-fake%3Afeature%2F%E7%95%8C") =>
                    {
                        (200, "[]".into())
                    }
                    p if p.starts_with("/user/repos?") => (
                        200,
                        r#"[{"name":"first","owner":{"login":"rocket-fake"},"default_branch":"main","html_url":"https://github.com/rocket-fake/first","description":"界面"},
                            {"name":"second","owner":{"login":"rocket-org"},"private":true,"default_branch":"main","html_url":"https://github.com/rocket-org/second"}]"#.into(),
                    ),
                    p if p.starts_with("/notifications?all=false") => (
                        200,
                        r#"[{"id":"1","unread":true,"reason":"review_requested","updated_at":"2026-10-21T00:00:00Z","subject":{"title":"Fix paths","url":"https://api.github.com/repos/rocket-org/second/pulls/42","type":"PullRequest"},"repository":{"name":"second","owner":{"login":"rocket-org"}}}]"#.into(),
                    ),
                    _ => (404, r#"{"message":"Not Found"}"#.into()),
                }
                });
                let ctx = Context::default();
                let mut app = Rocket::with_log_directory(
                    &ctx,
                    vec![f.repository.clone()],
                    f.root.join("logs"),
                );
                app.github_endpoint = Some(endpoint);
                app.opened_urls = Some(Vec::new());
                wait(&ctx, &mut app);
                app.auto_refresh = false;

                // Opening the page signs in without prompting and reads this branch's links.
                app.dispatch(Intent::Dialog(DialogKind::GitHub));
                wait(&ctx, &mut app);
                assert_eq!(app.dialog.as_ref().unwrap().error, None);
                assert_eq!(std::fs::read_to_string(&marker).unwrap().trim(), "never");
                assert_eq!(app.github.as_ref().unwrap().account().login, "rocket-fake");
                let links = page(&app).links.clone().expect("repository links");
                assert_eq!(links.remote, "origin");
                assert_eq!(links.pushed.as_deref(), Some("feature/界"));
                assert_eq!(links.ahead, 1);
                assert_eq!(links.base.full_name(), "rocket-org/upstream");
                assert_eq!(links.base_branch, "main");
                assert!(links.pull.is_none());

                // Create pull request records the fork-to-parent comparison page.
                click(&ctx, &mut app, "github-create-pull");
                click(&ctx, &mut app, "github-open-branch");
                assert_eq!(
                    app.opened_urls.as_deref().unwrap(),
                    [
                        "https://github.com/rocket-org/upstream/compare/main...rocket-fake:feature/%E7%95%8C?expand=1",
                        "https://github.com/rocket-fake/forked_repo/tree/feature/%E7%95%8C",
                    ]
                );

                // Notifications read once, filter, and open the subject page.
                click(&ctx, &mut app, "github-tab-notifications");
                wait(&ctx, &mut app);
                assert_eq!(page(&app).notifications.len(), 1);
                click(&ctx, &mut app, "github-notification-row-0");
                click(&ctx, &mut app, "github-notification-open");
                assert_eq!(
                    app.opened_urls
                        .as_deref()
                        .unwrap()
                        .last()
                        .map(String::as_str),
                    Some("https://github.com/rocket-org/second/pull/42")
                );

                // Repositories: the session is reused (no second credential lookup),
                // and Clone opens the Clone dialog with the repository's HTTPS URL.
                click(&ctx, &mut app, "github-tab-repositories");
                wait(&ctx, &mut app);
                assert_eq!(std::fs::read_to_string(&marker).unwrap().lines().count(), 1);
                let names: Vec<_> = page(&app)
                    .repositories
                    .iter()
                    .map(|r| r.remote.full_name())
                    .collect();
                assert_eq!(names, ["rocket-fake/first", "rocket-org/second"]);
                app.dialog.as_mut().unwrap().github.as_mut().unwrap().filter = "SECOND".into();
                assert_eq!(page(&app).visible_repositories(), [1]);
                click(&ctx, &mut app, "github-repository-row-1");
                click(&ctx, &mut app, "github-repository-clone");
                let dialog = app.dialog.as_ref().unwrap();
                assert!(matches!(dialog.kind, DialogKind::Clone));
                assert_eq!(dialog.first, "https://github.com/rocket-org/second.git");

                // Diagnostics name GitHub requests, never the token or account data.
                let logs: String = std::fs::read_dir(f.root.join("logs"))
                    .unwrap()
                    .filter_map(|e| std::fs::read_to_string(e.unwrap().path()).ok())
                    .collect();
                for label in [
                    "github-sign-in",
                    "github-repository",
                    "github-pulls",
                    "github-notifications",
                    "github-repositories",
                    "github-open",
                ] {
                    assert!(logs.contains(label), "{label} was not logged");
                }
                for private in [
                    "gho_rocketUiFake",
                    "rocket-fake",
                    "rocket-org",
                    "Fix paths",
                    "界",
                ] {
                    assert!(
                        !logs.contains(private),
                        "{private} reached the diagnostic log"
                    );
                }
            },
        );
    }

    #[test]
    fn refused_sign_in_offers_sign_in_and_no_remote_needs_none() {
        common::isolated(
            CASE,
            "hosting::flow_tests::refused_sign_in_offers_sign_in_and_no_remote_needs_none",
            || {
                let f = common::Fixture::new();
                let marker = helper(&f);
                f.write("file", "one");
                f.commit("one");
                let endpoint = serve(|_| (401, r#"{"message":"Bad credentials"}"#.into()));
                let ctx = Context::default();
                let mut app = Rocket::with_log_directory(
                    &ctx,
                    vec![f.repository.clone()],
                    f.root.join("logs"),
                );
                app.github_endpoint = Some(endpoint);
                app.opened_urls = Some(Vec::new());
                wait(&ctx, &mut app);
                app.auto_refresh = false;

                // Without a github.com remote the first section needs no sign-in.
                app.dispatch(Intent::Dialog(DialogKind::GitHub));
                wait(&ctx, &mut app);
                assert!(page(&app).no_remote);
                assert!(!marker.exists(), "the credential helper ran without need");

                // A refused credential is not kept, and Sign in is offered.
                click(&ctx, &mut app, "github-tab-repositories");
                wait(&ctx, &mut app);
                let error = app.dialog.as_ref().unwrap().error.clone().unwrap();
                assert!(error.contains("github_unauthorized"), "{error}");
                assert!(page(&app).sign_in && app.github.is_none());
                click(&ctx, &mut app, "github-sign-in");
                wait(&ctx, &mut app);
                assert_eq!(
                    std::fs::read_to_string(&marker)
                        .unwrap()
                        .lines()
                        .collect::<Vec<_>>(),
                    ["never", "auto"]
                );

                // Only github.com pages open; the page closes when its repository does.
                app.open_github("https://example.com/");
                assert!(app.opened_urls.as_deref().unwrap().is_empty());
                app.dialog.as_mut().unwrap().owner = Some(u64::MAX);
                frame(&ctx, &mut app, Vec::new());
                assert!(app.dialog.is_none());
            },
        );
    }
}
