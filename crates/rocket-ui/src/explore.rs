//! Explore: search GitHub's repositories, or paste any repository address, then
//! preview one read-only in a tab or clone it. A search runs only when asked (Enter
//! or Search): GitHub allows 30 searches a minute signed in and 10 without, so
//! typing alone sends nothing. Results stay when a search is refused or limited.
use crate::{
    app::*,
    backend::user_error,
    pages::{self, control},
    preview_tab::PreviewRequest,
    style::*,
};
use eframe::egui::*;
use rocket_git::*;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

/// The languages offered in the Language list; others can be typed as `language:`.
pub const LANGUAGES: [&str; 16] = [
    "C", "C#", "C++", "Go", "Java", "JavaScript", "Kotlin", "Lua", "PHP", "Python", "Ruby",
    "Rust", "Shell", "Swift", "TypeScript", "Zig",
];

/// Shown under the search box once this few searches are left.
const FEW_LEFT: u64 = 5;

/// Why the list shows a message instead of, or above, results.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Problem {
    Offline,
    /// GitHub's 422.
    Refused,
    NoResults,
    Failed(String),
}

struct Done {
    session: Arc<GitHubSession>,
    outcome: Result<GitHubSearchOutcome>,
}

#[derive(Default)]
pub struct State {
    /// What the box holds.
    pub query: String,
    /// The search the list shows: the box's text with the language added.
    pub searched: String,
    pub language: Option<&'static str>,
    pub sort: GitHubSearchSort,
    pub results: Vec<GitHubSearchItem>,
    pub total: u64,
    pub incomplete: bool,
    pub next_page: Option<u32>,
    pub selected: Option<usize>,
    /// Searches GitHub says are left this minute.
    pub remaining: Option<u64>,
    /// Rate limited until then; earlier results stay.
    pub wait_until: Option<Instant>,
    pub problem: Option<Problem>,
    /// The session searches use when there is no sign-in.
    pub anonymous: Option<Arc<GitHubSession>>,
    /// A silent sign-in was tried this run; don't keep asking the credential helper.
    pub tried_sign_in: bool,
    /// Preview anyway was offered for this result (bigger than the preview limit).
    pub confirm_large: Option<usize>,
    /// Put the keyboard in the search box on the next frame.
    pub focus: bool,
    task: Option<(ReadTask<Done>, bool)>,
}

impl State {
    pub fn running(&self) -> bool {
        self.task.is_some()
    }
    /// Seconds until searching is allowed again.
    pub fn wait_seconds(&self) -> Option<u64> {
        let until = self.wait_until?;
        let left = until.saturating_duration_since(Instant::now());
        (left > Duration::ZERO).then(|| left.as_secs() + u64::from(left.subsec_nanos() > 0))
    }
    pub fn cancel(&mut self) {
        self.task = None;
    }
}

/// A repository address typed or pasted into the box, previewable without a search.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Address {
    pub source: CloneSource,
    /// What origin should name once kept: the URL, or the folder.
    pub url: String,
    /// "owner/name", or the folder's name.
    pub name: String,
    pub host: String,
    /// A github.com page, for Open on GitHub.
    pub web: Option<String>,
}

/// The box's text as a repository address: an https:// or ssh:// URL, or an
/// existing local repository folder. Anything else is a search.
pub fn address(text: &str) -> Option<Address> {
    let text = text.trim();
    if text.is_empty() || text.chars().any(char::is_control) {
        return None;
    }
    if let Some(rest) = text.strip_prefix("https://").or_else(|| text.strip_prefix("ssh://")) {
        let (authority, path) = rest.split_once('/')?;
        let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
        let host = host.split(':').next().unwrap_or(host).to_ascii_lowercase();
        let mut parts = path.split('/').filter(|p| !p.is_empty());
        if host == "github.com" {
            // A pasted page such as …/owner/name/tree/main still means the repository.
            let owner = parts.next()?;
            let name = parts.next()?;
            let remote = GitHubRemote::new(owner, name.strip_suffix(".git").unwrap_or(name))?;
            return Some(Address {
                source: CloneSource::Url(format!("https://github.com/{}.git", remote.full_name())),
                url: format!("https://github.com/{}.git", remote.full_name()),
                name: remote.full_name(),
                host,
                web: Some(remote.web_url()),
            });
        }
        let segments: Vec<&str> = parts.collect();
        let name = match segments.as_slice() {
            [] => return None,
            [one] => one.strip_suffix(".git").unwrap_or(one).to_owned(),
            [.., owner, last] => format!("{owner}/{}", last.strip_suffix(".git").unwrap_or(last)),
        };
        if text.contains(['?', '#', '\\']) || text.contains(char::is_whitespace) {
            return None;
        }
        return Some(Address {
            source: CloneSource::Url(text.to_owned()),
            url: text.to_owned(),
            name,
            host,
            web: None,
        });
    }
    let path = Path::new(text);
    if path.is_absolute() && path.is_dir() {
        let name = path.file_name()?.to_string_lossy().into_owned();
        return Some(Address {
            source: CloneSource::Local(path.to_path_buf()),
            url: text.to_owned(),
            name,
            host: "this computer".into(),
            web: None,
        });
    }
    None
}

/// The query GitHub gets: the box's text and the chosen language.
pub fn compose(query: &str, language: Option<&str>) -> String {
    let query = query.trim();
    match language {
        Some(l) if !query.to_ascii_lowercase().contains("language:") => {
            let l = if l.contains(' ') { format!("\"{l}\"") } else { l.to_owned() };
            format!("{query} language:{l}")
        }
        _ => query.to_owned(),
    }
}

/// 18214 → "18.2k", 1_200_000 → "1.2M".
pub fn count(n: u64) -> String {
    match n {
        0..1_000 => n.to_string(),
        1_000..1_000_000 => trim(format!("{:.1}", n as f64 / 1_000.0)) + "k",
        _ => trim(format!("{:.1}", n as f64 / 1_000_000.0)) + "M",
    }
}
fn trim(text: String) -> String {
    text.strip_suffix(".0").map(str::to_owned).unwrap_or(text)
}

/// Bytes as "41 MB" or "4.9 GB".
pub fn size(bytes: u64) -> String {
    const MB: f64 = 1024.0 * 1024.0;
    let mb = bytes as f64 / MB;
    if mb < 1.0 {
        format!("{} KB", (bytes / 1024).max(1))
    } else if mb < 1024.0 {
        format!("{} MB", mb.round() as u64)
    } else {
        trim(format!("{:.1}", mb / 1024.0)) + " GB"
    }
}

/// GitHub's "2026-11-02T10:00:00Z" as Unix seconds.
pub fn unix(time: &str) -> Option<i64> {
    let (date, clock) = time.strip_suffix('Z')?.split_once('T')?;
    let mut d = date.split('-').map(|v| v.parse::<i64>().ok());
    let (y, m, day) = (d.next()??, d.next()??, d.next()??);
    let mut c = clock.split(':').map(|v| v.parse::<i64>().ok());
    let (h, min, s) = (c.next()??, c.next()??, c.next()??);
    // Days from the civil date (Howard Hinnant's algorithm).
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some((era * 146_097 + doe - 719_468) * 86_400 + h * 3_600 + min * 60 + s)
}

fn updated(time: Option<&str>) -> Option<String> {
    let at = unix(time?)?;
    let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs() as i64;
    Some(format!("Updated {}", ago(now.saturating_sub(at).max(0) as u64, at)))
}

#[derive(Default)]
struct Flags {
    search: bool,
    more: bool,
    stop: bool,
    sign_in: bool,
    close: bool,
    select: Option<usize>,
    preview: Option<(PreviewRequest, bool)>,
    clone: Option<String>,
    open: Option<String>,
    sort: Option<GitHubSearchSort>,
    language: Option<Option<&'static str>>,
    confirm: Option<usize>,
}

fn request_for(item: &GitHubSearchItem) -> PreviewRequest {
    PreviewRequest {
        source: if item.clone_url.starts_with("https://") || item.clone_url.starts_with("ssh://") {
            CloneSource::Url(item.clone_url.clone())
        } else {
            CloneSource::Local(PathBuf::from(&item.clone_url))
        },
        url: item.clone_url.clone(),
        name: item.remote.full_name(),
        host: "github.com".into(),
        web: Some(item.html_url.clone()),
        size_kb: Some(item.size_kb),
        branch: Some(item.default_branch.clone()),
    }
}

fn request_from(address: &Address) -> PreviewRequest {
    PreviewRequest {
        source: address.source.clone(),
        url: address.url.clone(),
        name: address.name.clone(),
        host: address.host.clone(),
        web: address.web.clone(),
        size_kb: None,
        branch: None,
    }
}

impl Rocket {
    /// Open Explore, optionally with text to search for or an address to show.
    pub(crate) fn open_explore(&mut self, query: Option<String>) {
        if !matches!(self.dialog.as_ref().map(|d| &d.kind), Some(DialogKind::Explore)) {
            self.show_dialog(DialogKind::Explore);
        }
        self.explore.focus = true;
        if let Some(query) = query {
            self.explore.query = query;
            if address(&self.explore.query).is_none() && !self.explore.query.trim().is_empty() {
                self.start_explore_search(false, false);
            }
        }
    }

    /// Run the box's search, or the next page of the shown one. Without a session,
    /// sign in silently once (or with the helper's window when `interactive`), and
    /// otherwise search public repositories without one.
    pub(crate) fn start_explore_search(&mut self, more: bool, interactive: bool) {
        let s = &mut self.explore;
        if s.wait_seconds().is_some() && !interactive {
            return;
        }
        let Some(endpoint) = self.github_endpoint else {
            s.problem = Some(Problem::Failed(user_error(&Error::Hosting(HostingFailure::Disabled))));
            return;
        };
        let Some(backend) = self.backend.clone() else {
            return;
        };
        let query = if more { s.searched.clone() } else { compose(&s.query, s.language) };
        if query.trim().is_empty() {
            return;
        }
        let page = if more { s.next_page.unwrap_or(1) } else { 1 };
        if !more {
            s.searched = query.clone();
            s.confirm_large = None;
        }
        s.problem = None;
        let sort = s.sort;
        let signed = self.github.clone();
        let anonymous = s.anonymous.clone();
        let try_sign_in = interactive || (!s.tried_sign_in && signed.is_none());
        s.tried_sign_in |= try_sign_in;
        match ReadTask::spawn(move |cancel| {
            let session = match (signed, try_sign_in) {
                (Some(session), false) => session,
                (_, true) => match backend.github_session(endpoint, interactive, &cancel) {
                    Ok(session) => Arc::new(session),
                    Err(Error::Hosting(
                        HostingFailure::SignInRequired | HostingFailure::Unauthorized,
                    )) if !interactive => match anonymous {
                        Some(session) => session,
                        None => Arc::new(backend.github_anonymous(endpoint)?),
                    },
                    Err(error) => return Err(error),
                },
                (None, false) => match anonymous {
                    Some(session) => session,
                    None => Arc::new(backend.github_anonymous(endpoint)?),
                },
            };
            let outcome = backend.github("github-search", &session, |g| {
                g.search_repositories(&query, sort, page, &cancel)
            });
            Ok(Done { session, outcome })
        }) {
            Ok(task) => s.task = Some((task, more)),
            Err(error) => s.problem = Some(Problem::Failed(user_error(&error))),
        }
    }

    pub(crate) fn poll_explore(&mut self, ctx: &Context) {
        let s = &mut self.explore;
        if s.wait_until.is_some() {
            if s.wait_seconds().is_some() {
                ctx.request_repaint_after(Duration::from_millis(250));
            } else {
                s.wait_until = None;
            }
        }
        let Some(result) = s.task.as_mut().and_then(|(task, _)| task.try_finish()) else {
            return;
        };
        let more = s.task.take().is_some_and(|(_, more)| more);
        let Done { session, outcome } = match result {
            Ok(done) => done,
            Err(Error::Cancelled) => return,
            Err(error) => {
                s.problem = Some(problem(&error));
                return;
            }
        };
        if session.signed_in() {
            self.github = Some(session);
        } else {
            s.anonymous = Some(session);
        }
        let s = &mut self.explore;
        match outcome {
            Ok(GitHubSearchOutcome::Page(page)) => {
                if !more {
                    s.results.clear();
                    s.selected = None;
                }
                for item in page.items {
                    if !s.results.iter().any(|r| r.remote == item.remote) {
                        s.results.push(item);
                    }
                }
                if !more && !s.results.is_empty() {
                    s.selected = Some(0);
                }
                s.total = page.total;
                s.incomplete = page.incomplete;
                s.next_page = page.next_page;
                s.remaining = page.remaining;
                s.problem = s.results.is_empty().then_some(Problem::NoResults);
            }
            Ok(GitHubSearchOutcome::RateLimited { reset_at, retry_after }) => {
                let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
                let wait = retry_after
                    .or_else(|| reset_at.map(|r| r.saturating_sub(now)))
                    .unwrap_or(60)
                    .clamp(1, 3_600);
                s.wait_until = Some(Instant::now() + Duration::from_secs(wait));
                s.remaining = Some(0);
            }
            Ok(GitHubSearchOutcome::Refused) => s.problem = Some(Problem::Refused),
            Err(error) => {
                if matches!(error, Error::Hosting(HostingFailure::Unauthorized)) {
                    // The stored sign-in stopped working: search without it next time.
                    self.github = None;
                }
                self.explore.problem = Some(problem(&error));
            }
        }
    }

    pub(crate) fn explore_page(&mut self, ui: &mut Ui) {
        let Some(backend_ready) = self.backend.as_ref().map(|_| true) else {
            return;
        };
        let can_clone = backend_ready && self.write.is_none();
        let signed = self.github.as_ref().map(|g| g.account().login.clone());
        let limit = self.preview_limit;
        let mut f = Flags::default();
        let widgets = &mut self.frame_widgets;
        let s = &mut self.explore;
        let running = s.task.is_some();
        pages::header_with(
            ui,
            |painter, tile| icon(painter, tile.shrink(10.0), Icon::Compass, BLUE()),
            "Explore",
            "Search GitHub, or paste any repository address to preview it",
            |ui| {
                let close = pages::ghost(ui, true, "Close");
                control(widgets, "explore-close", &close, ui.clip_rect());
                f.close = close.clicked();
                match &signed {
                    Some(login) => {
                        ui.label(RichText::new(format!("Signed in as {login}")).color(MUTED()));
                        dot(ui, GREEN());
                    }
                    None => {
                        let r = ui.add_enabled(!running, Button::new(RichText::new("Sign in").color(BLUE())).frame(false));
                        control(widgets, "explore-sign-in", &r, ui.clip_rect());
                        f.sign_in = r.on_hover_text("Uses the github.com sign-in Git stores; it adds private repositories and allows 30 searches a minute").clicked();
                        ui.label(RichText::new("Not signed in · public repositories").color(MUTED()));
                        dot(ui, HINT());
                    }
                }
            },
        );
        // The search box, language and order.
        let typed = address(&s.query);
        Frame::new()
            .inner_margin(Margin { left: 24, right: 24, top: 0, bottom: 10 })
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                // The order and language controls take their own width from the right;
                // the box gets the rest, so nothing pushes the page past the window.
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        for (sort, label, key) in [
                            (GitHubSearchSort::Updated, "Updated", "explore-sort-updated"),
                            (GitHubSearchSort::Stars, "Stars", "explore-sort-stars"),
                            (GitHubSearchSort::BestMatch, "Best match", "explore-sort-best"),
                        ] {
                            let r = pages::choice(ui, true, s.sort == sort, label);
                            control(widgets, key, &r, ui.clip_rect());
                            if r.clicked() && s.sort != sort {
                                f.sort = Some(sort);
                            }
                        }
                        let mut language = s.language;
                        ComboBox::from_id_salt("explore-language")
                            .selected_text(format!("Language: {}", language.unwrap_or("Any")))
                            .width(150.0)
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut language, None, "Any");
                                for l in LANGUAGES {
                                    ui.selectable_value(&mut language, Some(l), l);
                                }
                            });
                        if language != s.language {
                            f.language = Some(language);
                        }
                        let waiting = s.wait_seconds();
                        let search = ui.add_enabled(
                            !running && waiting.is_none() && typed.is_none() && !s.query.trim().is_empty(),
                            Button::new("Search").min_size(vec2(0.0, 32.0)),
                        );
                        control(widgets, "explore-search", &search, ui.clip_rect());
                        f.search |= search.clicked();
                        let edit = ui.add_sized(
                            vec2(ui.available_width().max(160.0), 32.0),
                            TextEdit::singleline(&mut s.query)
                                .hint_text("Search GitHub or paste a repository address")
                                .align(Align2::LEFT_CENTER)
                                .margin(Margin::symmetric(10, 6)),
                        );
                        control(widgets, "explore-query", &edit, ui.clip_rect());
                        if std::mem::take(&mut s.focus) {
                            edit.request_focus();
                        }
                        if edit.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) && typed.is_none() {
                            f.search = true;
                        }
                    });
                });
                // One line under the box: counts, limits and problems.
                ui.add_space(8.0);
                if let Some(seconds) = s.wait_seconds() {
                    pages::callout(
                        ui,
                        AMBER(),
                        &format!(
                            "GitHub's search limit is reached. You can search again in {}:{:02}.",
                            seconds / 60,
                            seconds % 60
                        ),
                    );
                } else if let Some(problem) = &s.problem {
                    match problem {
                        Problem::Refused => pages::callout(ui, RED(), "GitHub can't run this search. Check user: and org: names, and use at most five AND, OR or NOT."),
                        Problem::Failed(text) => pages::callout(ui, RED(), text),
                        Problem::Offline | Problem::NoResults => {}
                    }
                }
                if typed.is_none() && !s.results.is_empty() {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(format!("{} repositories", s.total.min(9_999_999))).color(TEXT()));
                        ui.label(RichText::new(format!("· showing {}", s.results.len())).color(HINT()));
                        if s.incomplete {
                            ui.label(RichText::new("· GitHub stopped early; there may be more").color(HINT()));
                        }
                        if let Some(left) = s.remaining.filter(|l| *l <= FEW_LEFT && s.wait_until.is_none()) {
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                ui.label(RichText::new(format!("{left} searches left this minute")).size(12.0).color(HINT()));
                            });
                        }
                    });
                }
            });
        if let Some(address) = &typed {
            address_card(ui, address, can_clone, widgets, &mut f);
        } else {
            pages::split(ui, |ui, side| match side {
                pages::Side::List => results_list(ui, s, Some(limit), running, widgets, &mut f),
                pages::Side::Details => details(ui, s, Some(limit), can_clone, widgets, &mut f),
            });
        }
        pages::details_scroll(ui, widgets, true);
        self.finish_explore_frame(f);
    }

    fn finish_explore_frame(&mut self, f: Flags) {
        if f.close {
            self.explore.cancel();
            self.dialog = None;
            return;
        }
        if let Some(i) = f.select {
            self.explore.selected = Some(i);
            self.explore.confirm_large = None;
        }
        if let Some(i) = f.confirm {
            self.explore.confirm_large = Some(i);
        }
        if let Some(language) = f.language {
            self.explore.language = language;
            if !self.explore.query.trim().is_empty() {
                self.start_explore_search(false, false);
            }
        }
        if let Some(sort) = f.sort {
            self.explore.sort = sort;
            if !self.explore.query.trim().is_empty() {
                self.start_explore_search(false, false);
            }
        }
        if f.stop {
            self.explore.cancel();
        } else if f.sign_in {
            self.start_explore_search(false, true);
        } else if f.search {
            self.start_explore_search(false, false);
        } else if f.more {
            self.start_explore_search(true, false);
        }
        if let Some(url) = f.open {
            self.open_github(&url);
        }
        if let Some(url) = f.clone {
            self.show_dialog(DialogKind::Clone);
            if let Some(d) = &mut self.dialog {
                d.first = url;
            }
            return;
        }
        if let Some((request, unlimited)) = f.preview {
            self.start_preview(request, unlimited);
        }
    }
}

fn problem(error: &Error) -> Problem {
    match error {
        Error::Hosting(HostingFailure::Network) => Problem::Offline,
        other => Problem::Failed(user_error(other)),
    }
}

fn dot(ui: &mut Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(vec2(7.0, 7.0), Sense::hover());
    ui.painter().circle_filled(rect.center(), 3.5, color);
}

/// One result: owner and name, pills, description, then stars, language, last push
/// and license, in a 70-point row.
fn result_row(ui: &mut Ui, item: &GitHubSearchItem, selected: bool, limit: Option<u64>) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 70.0), Sense::click());
    let painter = ui.painter_at(rect);
    if selected {
        painter.rect_filled(rect, 8.0, SELECTED());
        painter.rect_filled(Rect::from_min_size(rect.min + vec2(0.0, 12.0), vec2(3.0, rect.height() - 24.0)), 1.5, BLUE());
    } else if response.hovered() {
        painter.rect_filled(rect, 8.0, HOVER());
    }
    let tile = Rect::from_min_size(rect.min + vec2(12.0, 12.0), vec2(30.0, 30.0));
    painter.rect_filled(tile, 7.0, RAISED());
    let initial = item.remote.owner.chars().next().unwrap_or('?').to_uppercase().to_string();
    painter.text(tile.center(), Align2::CENTER_CENTER, initial, semibold(12.0), MUTED());
    let left = tile.right() + 12.0;
    let width = rect.right() - left - 12.0;
    let mut job = text::LayoutJob::default();
    job.append(&format!("{} / ", item.remote.owner), 0.0, TextFormat::simple(FontId::proportional(13.0), MUTED()));
    job.append(&item.remote.name, 0.0, TextFormat::simple(semibold(13.0), TEXT()));
    job.wrap = text::TextWrapping::truncate_at_width(width * 0.7);
    let title = painter.layout_job(job);
    let title_width = title.size().x;
    painter.galley(pos2(left, rect.top() + 9.0), title, TEXT());
    let mut x = left + title_width + 8.0;
    let mut pills: Vec<(String, Color32)> = Vec::new();
    if item.private {
        pills.push(("Private".into(), AMBER()));
    }
    if limit.is_some_and(|l| item.size_kb.saturating_mul(1024) > l) {
        pills.push((size(item.size_kb * 1024), AMBER()));
    }
    if item.archived {
        pills.push(("Archived".into(), FAINT()));
    }
    if item.fork {
        pills.push(("Fork".into(), VIOLET()));
    }
    for (text, color) in pills {
        let galley = painter.layout_no_wrap(text, semibold(11.0), color);
        let pill = Rect::from_min_size(pos2(x, rect.top() + 9.0), vec2(galley.size().x + 14.0, 18.0));
        if pill.right() > rect.right() - 8.0 {
            break;
        }
        painter.rect_filled(pill, 9.0, color.gamma_multiply(0.16));
        painter.galley(pos2(pill.left() + 7.0, pill.center().y - galley.size().y / 2.0), galley, color);
        x = pill.right() + 6.0;
    }
    let description = item.description.as_deref().unwrap_or("No description");
    let mut job = text::LayoutJob::simple_singleline(
        description.into(),
        FontId::proportional(12.5),
        if item.description.is_some() { TEXT() } else { HINT() },
    );
    job.wrap = text::TextWrapping::truncate_at_width(width);
    painter.galley(pos2(left, rect.top() + 29.0), painter.layout_job(job), TEXT());
    // Stars, language, last push, license.
    let mut x = left;
    let y = rect.top() + 49.0;
    let star = Rect::from_min_size(pos2(x, y), vec2(13.0, 13.0));
    icon(&painter, star, Icon::Star, FAINT());
    x += 17.0;
    let meta = |painter: &Painter, x: &mut f32, text: String| {
        let galley = painter.layout_no_wrap(text, FontId::proportional(12.0), FAINT());
        let w = galley.size().x;
        painter.galley(pos2(*x, y), galley, FAINT());
        *x += w + 16.0;
    };
    meta(&painter, &mut x, count(item.stars));
    if let Some(language) = &item.language {
        painter.circle_filled(pos2(x + 4.0, y + 7.0), 4.5, language_color(language));
        x += 13.0;
        meta(&painter, &mut x, language.clone());
    }
    if let Some(when) = updated(item.pushed_at.as_deref()) {
        meta(&painter, &mut x, when);
    }
    if let Some(license) = &item.license {
        meta(&painter, &mut x, license.clone());
    }
    response
}

/// GitHub's usual colors for the common languages; others get a neutral dot.
fn language_color(language: &str) -> Color32 {
    match language {
        "Go" => Color32::from_rgb(0x00, 0xAD, 0xD8),
        "Rust" => Color32::from_rgb(0xDE, 0xA5, 0x84),
        "Python" => Color32::from_rgb(0x35, 0x72, 0xA5),
        "TypeScript" => Color32::from_rgb(0x31, 0x78, 0xC6),
        "JavaScript" => Color32::from_rgb(0xF1, 0xE0, 0x5A),
        "C++" => Color32::from_rgb(0xF3, 0x4B, 0x7D),
        "C" => Color32::from_rgb(0x55, 0x55, 0x55),
        "C#" => Color32::from_rgb(0x17, 0x86, 0x00),
        "Java" => Color32::from_rgb(0xB0, 0x72, 0x19),
        "Lua" => Color32::from_rgb(0x7B, 0x83, 0xF5),
        "Shell" => Color32::from_rgb(0x89, 0xE0, 0x51),
        "Ruby" => Color32::from_rgb(0x70, 0x15, 0x16),
        "Kotlin" => Color32::from_rgb(0xA9, 0x7B, 0xFF),
        "Swift" => Color32::from_rgb(0xF0, 0x51, 0x38),
        "PHP" => Color32::from_rgb(0x4F, 0x5D, 0x95),
        "HTML" => Color32::from_rgb(0xE3, 0x4C, 0x26),
        _ => FAINT(),
    }
}

fn results_list(
    ui: &mut Ui,
    s: &State,
    limit: Option<u64>,
    running: bool,
    widgets: &mut HashMap<String, UiControl>,
    f: &mut Flags,
) {
    if s.results.is_empty() {
        if running {
            pages::loading_with(ui, "Searching GitHub", 40.0, |ui| {
                let stop = ui.button("Stop");
                control(widgets, "explore-stop", &stop, ui.clip_rect());
                f.stop = stop.clicked();
            });
            return;
        }
        match &s.problem {
            Some(Problem::Offline) => {
                pages::loading_with(ui, "Can't reach GitHub", 40.0, |ui| {
                    ui.label(RichText::new("Check your connection, then search again.").color(MUTED()));
                    let again = ui.button("Try again");
                    control(widgets, "explore-retry", &again, ui.clip_rect());
                    f.search = again.clicked();
                });
                return;
            }
            Some(Problem::NoResults) => {
                pages::empty(ui, "No repositories match. Try fewer words, or remove a qualifier.");
                return;
            }
            _ => {}
        }
        if s.searched.is_empty() {
            pages::empty(ui, "Search GitHub's repositories, or paste an address from any host.");
        }
        return;
    }
    let footer = 44.0;
    let height = (ui.available_height() - footer).max(80.0);
    let output = ScrollArea::vertical()
        .id_salt("explore-results")
        .max_height(height)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            for (i, item) in s.results.iter().enumerate() {
                let r = result_row(ui, item, s.selected == Some(i), limit)
                    .on_hover_cursor(CursorIcon::PointingHand);
                control(widgets, format!("explore-result-{i}"), &r, ui.clip_rect());
                if r.clicked() {
                    f.select = Some(i);
                }
            }
        });
    pages::region(widgets, "explore-results-scroll", output.inner_rect.intersect(ui.clip_rect()), true);
    ui.horizontal(|ui| {
        ui.set_min_height(36.0);
        let shown = s.results.len() as u64;
        let capped = s.next_page.is_none() && shown >= GITHUB_SEARCH_LIMIT.min(s.total) && s.total > GITHUB_SEARCH_LIMIT;
        let text = if capped {
            format!("{shown} of {} loaded · GitHub stops here. Narrow the search to see others.", s.total)
        } else {
            format!("{shown} of {} loaded", s.total.max(shown))
        };
        ui.label(RichText::new(text).size(12.0).color(HINT()));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if running {
                let stop = ui.button("Stop");
                control(widgets, "explore-stop", &stop, ui.clip_rect());
                f.stop = stop.clicked();
                ui.spinner();
            } else {
                let more = ui.add_enabled(s.next_page.is_some() && s.wait_until.is_none(), Button::new("Load more"));
                control(widgets, "explore-more", &more, ui.clip_rect());
                f.more = more.clicked();
            }
        });
    });
}

fn details(
    ui: &mut Ui,
    s: &State,
    limit: Option<u64>,
    can_clone: bool,
    widgets: &mut HashMap<String, UiControl>,
    f: &mut Flags,
) {
    let Some((index, item)) = s.selected.and_then(|i| s.results.get(i).map(|r| (i, r))) else {
        pages::empty(ui, "Select a repository to see it here.");
        return;
    };
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(format!("{} / ", item.remote.owner)).size(15.0).color(MUTED()));
        ui.label(RichText::new(&item.remote.name).font(semibold(15.0)));
    });
    if let Some(description) = &item.description {
        ui.label(RichText::new(description).color(TEXT()));
    }
    ui.add_space(4.0);
    // Wraps in a narrow window instead of widening the card.
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let (rect, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
        icon(ui.painter(), rect, Icon::Star, AMBER());
        ui.label(RichText::new(format!("{} stars", group(item.stars))).color(TEXT()));
        ui.label(RichText::new(format!("· {} forks · {} open issues", group(item.forks), group(item.open_issues))).color(MUTED()));
    });
    ui.add_space(6.0);
    pages::field(ui, "Language", item.language.as_deref().unwrap_or("Not stated"), false);
    pages::field(ui, "License", item.license.as_deref().unwrap_or("None stated"), false);
    pages::field(ui, "Default branch", &item.default_branch, false);
    pages::field(ui, "Size on GitHub", &size(item.size_kb.saturating_mul(1024)), false);
    if let Some(pushed) = &item.pushed_at {
        let shown = updated(Some(pushed)).map_or_else(|| pushed.clone(), |u| format!("{} · {}", u.trim_start_matches("Updated "), &pushed[..10.min(pushed.len())]));
        pages::field(ui, "Last push", &shown, false);
    }
    if !item.topics.is_empty() {
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
            for topic in &item.topics {
                pages::pill(ui, topic, BLUE());
            }
        });
    }
    ui.add_space(10.0);
    let large = limit.is_some_and(|l| item.size_kb.saturating_mul(1024) > l);
    if large && s.confirm_large == Some(index) {
        pages::callout(
            ui,
            AMBER(),
            &format!(
                "About {} on GitHub. Previews stop at {}, and its history alone may pass that.",
                size(item.size_kb.saturating_mul(1024)),
                size(limit.unwrap_or(PREVIEW_LIMIT))
            ),
        );
        ui.horizontal_wrapped(|ui| {
            let anyway = pages::ghost(ui, true, "Preview anyway");
            control(widgets, "explore-preview-anyway", &anyway, ui.clip_rect());
            if anyway.clicked() {
                f.preview = Some((request_for(item), true));
            }
            let clone = pages::primary(ui, can_clone, "Clone instead");
            control(widgets, "explore-clone-instead", &clone, ui.clip_rect());
            if clone.clicked() {
                f.clone = Some(item.clone_url.clone());
            }
        });
        return;
    }
    ui.horizontal_wrapped(|ui| {
        let preview = pages::primary(ui, true, "Preview").on_hover_text("Opens a read-only copy in a new tab");
        control(widgets, "explore-preview", &preview, ui.clip_rect());
        if preview.clicked() {
            if large {
                f.confirm = Some(index);
            } else {
                f.preview = Some((request_for(item), false));
            }
        }
        let clone = pages::ghost(ui, can_clone, "Clone");
        control(widgets, "explore-clone", &clone, ui.clip_rect());
        if clone.clicked() {
            f.clone = Some(item.clone_url.clone());
        }
        let open = ui.add(Button::new(RichText::new("Open on GitHub").color(BLUE())).frame(false));
        control(widgets, "explore-open", &open, ui.clip_rect());
        if open.clicked() {
            f.open = Some(item.html_url.clone());
        }
    });
}

/// 18214 → "18,214".
pub(crate) fn group(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// A typed or pasted address: one card with Preview and Clone, any host.
fn address_card(
    ui: &mut Ui,
    address: &Address,
    can_clone: bool,
    widgets: &mut HashMap<String, UiControl>,
    f: &mut Flags,
) {
    Frame::new()
        .inner_margin(Margin { left: 24, right: 24, top: 4, bottom: 12 })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            Frame::new()
                .fill(PANEL())
                .stroke(Stroke::new(1.0, HAIRLINE()))
                .corner_radius(10)
                .inner_margin(Margin::symmetric(16, 14))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        let (tile, _) = ui.allocate_exact_size(vec2(30.0, 30.0), Sense::hover());
                        ui.painter().rect_filled(tile, 7.0, RAISED());
                        icon(ui.painter(), tile.shrink(8.0), Icon::Link, MUTED());
                        ui.add_space(6.0);
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 2.0;
                            ui.label(RichText::new(&address.name).font(semibold(14.0)));
                            ui.label(RichText::new(format!("{} · size unknown until it downloads", address.host)).size(12.0).color(HINT()));
                        });
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let clone = pages::ghost(ui, can_clone, "Clone");
                            control(widgets, "explore-address-clone", &clone, ui.clip_rect());
                            if clone.clicked() {
                                f.clone = Some(address.url.clone());
                            }
                            let preview = pages::primary(ui, true, "Preview");
                            control(widgets, "explore-address-preview", &preview, ui.clip_rect());
                            if preview.clicked() {
                                f.preview = Some((request_from(address), false));
                            }
                        });
                    });
                });
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Explore replaces the content area, with tabs open or not: its controls are
    /// drawn and the repository view's are not.
    #[test]
    fn explore_replaces_the_content_area() {
        crate::test_common::isolated(
            "ROCKET_EXPLORE_UI_CASE",
            "explore::tests::explore_replaces_the_content_area",
            || {
                let fixture = crate::test_common::Fixture::new();
                let ctx = Context::default();
                let mut app = Rocket::with_storage(&ctx, vec![fixture.repository.clone()], fixture.root.join("logs"), fixture.root.join("state"));
                let deadline = Instant::now() + Duration::from_secs(40);
                while app.busy() || app.tab().is_some_and(|t| t.snapshot.is_none()) {
                    app.poll(&ctx);
                    assert!(Instant::now() < deadline, "repository loads");
                    std::thread::sleep(Duration::from_millis(10));
                }
                let frame = |app: &mut Rocket| {
                    app.frame_widgets.clear();
                    let bounds = Rect::from_min_size(Pos2::ZERO, vec2(1480.0, 920.0));
                    ctx.run_ui(RawInput { screen_rect: Some(bounds), ..Default::default() }, |ui| app.shell(ui, &mut Vec::new()))
                        .drop_without_applying_deltas();
                };
                frame(&mut app);
                assert!(!app.frame_widgets.contains_key("explore-query"));
                // No query: a query searches at once, and tests never reach GitHub.
                app.github_endpoint = None;
                app.dispatch(Intent::Explore(None));
                frame(&mut app);
                assert!(app.frame_widgets.get("explore-query").is_some_and(|c| c.enabled));
                assert!(app.frame_widgets.contains_key("explore-search"));
                assert!(!app.frame_widgets.contains_key("search-kind"), "the commit list is hidden");
                assert!(app.explore.searched.is_empty() && !app.explore.running(), "opening doesn't search");
            },
        );
    }

    /// Results, the details pane and every control stay inside the window, from the
    /// narrowest supported width up: an overflowing search row once pushed the details
    /// off-screen, and the two columns were 8 points wider than the window.
    #[test]
    fn explore_fits_the_window() {
        crate::test_common::isolated(
            "ROCKET_EXPLORE_UI_CASE",
            "explore::tests::explore_fits_the_window",
            || {
                let fixture = crate::test_common::Fixture::new();
                let ctx = Context::default();
                let mut app = Rocket::with_storage(&ctx, vec![fixture.repository.clone()], fixture.root.join("logs"), fixture.root.join("state"));
                let deadline = Instant::now() + Duration::from_secs(40);
                while app.busy() || app.tab().is_some_and(|t| t.snapshot.is_none()) {
                    app.poll(&ctx);
                    assert!(Instant::now() < deadline, "repository loads");
                    std::thread::sleep(Duration::from_millis(10));
                }
                app.github_endpoint = None;
                // What the window's own chrome lays out at each width, without Explore.
                let mut baseline = Vec::new();
                for width in [780.0, 1100.0, 1480.0] {
                    let bounds = Rect::from_min_size(Pos2::ZERO, vec2(width, 920.0));
                    for _ in 0..2 {
                        app.frame_widgets.clear();
                        ctx.run_ui(RawInput { screen_rect: Some(bounds), ..Default::default() }, |ui| app.shell(ui, &mut Vec::new()))
                            .drop_without_applying_deltas();
                    }
                    baseline.push(ctx.globally_used_rect().right());
                }
                app.dispatch(Intent::Explore(None));
                let item = |name: &str, stars: u64| GitHubSearchItem {
                    remote: GitHubRemote::new("acme-tools", name).unwrap(),
                    description: Some("A fast fuzzy finder for the terminal, with a description long enough to wrap".into()),
                    stars,
                    forks: 812,
                    open_issues: 37,
                    language: Some("Rust".into()),
                    license: Some("MIT".into()),
                    topics: vec!["cli".into(), "search".into(), "fuzzy".into()],
                    default_branch: "main".into(),
                    size_kb: 5_000_000,
                    pushed_at: Some("2026-11-02T10:00:00Z".into()),
                    updated_at: Some("2026-11-02T10:00:00Z".into()),
                    private: true,
                    fork: true,
                    archived: true,
                    html_url: format!("https://github.com/acme-tools/{name}"),
                    clone_url: format!("https://github.com/acme-tools/{name}.git"),
                };
                app.explore.query = "fuzzy finder".into();
                app.explore.searched = "fuzzy finder".into();
                app.explore.results = (0..30).map(|i| item(&format!("finder-with-a-long-name-{i}"), 18_214 - i)).collect();
                app.explore.total = 2_184;
                app.explore.next_page = Some(2);
                app.explore.selected = Some(0);
                app.explore.remaining = Some(4);
                for (width, chrome) in [780.0, 1100.0, 1480.0].into_iter().zip(baseline) {
                    let bounds = Rect::from_min_size(Pos2::ZERO, vec2(width, 920.0));
                    for _ in 0..2 {
                        app.frame_widgets.clear();
                        ctx.run_ui(RawInput { screen_rect: Some(bounds), ..Default::default() }, |ui| app.shell(ui, &mut Vec::new()))
                            .drop_without_applying_deltas();
                    }
                    for key in ["explore-query", "explore-search", "explore-sort-updated", "explore-preview", "explore-clone", "explore-close", "explore-sign-in", "explore-more"] {
                        assert!(app.frame_widgets.contains_key(key), "{key} at {width}");
                    }
                    // Control rectangles are clipped to the window, so check what was laid out.
                    let used = ctx.globally_used_rect();
                    assert!(chrome <= width + 0.5, "the window without Explore fits at {width}");
                    assert!(used.right() <= width + 0.5, "Explore lays out to {} in a {width} window", used.right());
                }
            },
        );
    }

    #[test]
    fn addresses_are_told_from_searches() {
        let gh = address("https://github.com/Acme/Tool/tree/main/src").unwrap();
        assert_eq!(gh.name, "Acme/Tool");
        assert_eq!(gh.url, "https://github.com/Acme/Tool.git");
        assert_eq!(gh.web.as_deref(), Some("https://github.com/Acme/Tool"));
        let lab = address("https://gitlab.example.com/group/sub/tool.git").unwrap();
        assert_eq!((lab.name.as_str(), lab.host.as_str()), ("sub/tool", "gitlab.example.com"));
        assert!(matches!(lab.source, CloneSource::Url(_)));
        let ssh = address("ssh://git@host.example:2222/team/app.git").unwrap();
        assert_eq!((ssh.name.as_str(), ssh.host.as_str()), ("team/app", "host.example"));
        for search in ["fuzzy finder", "language:rust stars:>100", "https://", "https://host", "C:\\missing\\folder"] {
            assert!(address(search).is_none(), "{search}");
        }
        assert!(address("https://host.example/a b").is_none());
    }

    #[test]
    fn queries_counts_sizes_and_times_read_as_words() {
        assert_eq!(compose(" fuzzy ", Some("Rust")), "fuzzy language:Rust");
        assert_eq!(compose("fuzzy language:go", Some("Rust")), "fuzzy language:go");
        assert_eq!(compose("x", None), "x");
        assert_eq!((count(640), count(18_214), count(1_000), count(1_260_000)), ("640".into(), "18.2k".into(), "1k".into(), "1.3M".into()));
        assert_eq!(group(18_214), "18,214");
        assert_eq!((size(500), size(41 * 1024 * 1024), size(5_261_334_938)), ("1 KB".into(), "41 MB".into(), "4.9 GB".into()));
        assert_eq!(unix("1970-01-02T00:00:01Z"), Some(86_401));
        assert_eq!(unix("2026-11-02T10:00:00Z"), Some(1_793_613_600));
        assert_eq!(unix("bad"), None);
    }
}
