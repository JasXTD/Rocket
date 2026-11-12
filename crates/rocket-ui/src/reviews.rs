//! Independent native read-only windows. Each owns its selection and read receivers.
use crate::{
    app::{Intent, Rocket, Tab, UiControl, View},
    backend::{Backend, Snapshot, user_error},
    file_history::{self, FileIntent, Mode},
    pages,
    style::*,
};
use eframe::egui::*;
use rocket_git::*;
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};

#[derive(Clone)]
pub struct Request {
    pub commit: ObjectId,
    pub path: Option<RepoPath>,
    pub blame: bool,
}
pub type Window = Arc<Mutex<Review>>;
pub struct Review {
    pub id: u64,
    pub request: Request,
    pub tab: Tab,
    pub widgets: HashMap<String, UiControl>,
    pub size: [f32; 2],
    pub closed: bool,
    pub error: Option<String>,
    pub notice: Option<String>,
    pub screenshots: Vec<Event>,
    backend: Backend,
    repository: Repository,
    read: Option<ReadTask<HistoryPage>>,
}
impl Review {
    pub fn viewport(&self) -> ViewportId {
        ViewportId::from_hash_of(("rocket-review", self.id))
    }
    pub fn running(&self) -> bool {
        self.read.is_some()
            || self.tab.diff.is_some()
            || self.tab.file_history.running()
            || self.tab.image.running()
    }
    fn new(path: PathBuf, repository: Repository, backend: Backend, request: Request) -> Self {
        let mut tab = Tab::new(path, Some(backend.log.clone()));
        tab.review_window = true;
        tab.selected_commit = Some(request.commit.clone());
        tab.view = if request.path.is_some() {
            View::FileHistory
        } else {
            View::Commits
        };
        let mut value = Self {
            id: tab.id,
            request,
            tab,
            widgets: Default::default(),
            size: [0.0; 2],
            closed: false,
            error: None,
            notice: None,
            screenshots: Vec::new(),
            backend,
            repository,
            read: None,
        };
        value.load();
        value
    }
    fn load(&mut self) {
        self.error = None;
        self.notice = None;
        self.tab.image.clear();
        let backend = self.backend.clone();
        let repo = self.repository.clone();
        let id = self.request.commit.clone();
        match ReadTask::spawn(move |cancel| {
            backend.log.request("review-window", |log| {
                repo.with_log(log.clone())
                    .history(HistoryStart::Commit(id), 1, &cancel)
            })
        }) {
            Ok(task) => self.read = Some(task),
            Err(e) => self.error = Some(user_error(&e)),
        }
    }
    pub fn cancel(&mut self) {
        if let Some(t) = &self.read {
            t.cancel();
        }
        if let Some(t) = &self.tab.diff {
            t.cancel();
        }
        self.tab.file_history.stop();
        self.tab.image.stop();
        self.tab.diff_tools.cancel();
        self.notice = Some("Stopping read. Previous content stays available.".into());
    }
    pub fn close(&mut self) {
        self.cancel();
        self.closed = true;
        self.read = None;
        self.tab.diff = None;
        self.tab.file_history = Default::default();
        self.tab.diff_tools.cancel();
        self.tab.image.clear();
        let _ = self.backend.log.request("review-close", |_| Ok(()));
    }
    pub fn poll(&mut self, ctx: &Context) {
        if self.closed {
            return;
        }
        if let Some(result) = self.read.as_mut().and_then(ReadTask::try_finish) {
            self.read = None;
            match result {
                Ok(page) => {
                    self.tab.snapshot = Some(Snapshot {
                        watcher: None,
                        refresh_guard: None,
                        info: self.repository.info().clone(),
                        repository: self.repository.clone(),
                        head: None,
                        status: None,
                        branches: vec![],
                        remotes: vec![],
                        stashes: vec![],
                        state: None,
                        sequence: None,
                        history: Some(page),
                        warnings: vec![],
                        // A pinned review is read-only: no notice about its history.
                        ordering: None,
                        history_notice: false,
                        slow_loads: 0,
                    });
                    if let Some(path) = self.request.path.clone() {
                        self.tab.file_history.mode = if self.request.blame {
                            Mode::Blame
                        } else {
                            Mode::Diff
                        };
                        self.tab.file_history.load(
                            self.backend.clone(),
                            self.repository.clone(),
                            Some((
                                self.request.commit.clone(),
                                path,
                                self.tab.file_history.directory_input,
                            )),
                        );
                    } else {
                        let backend = self.backend.clone();
                        let repo = self.repository.clone();
                        let id = self.request.commit.clone();
                        match ReadTask::spawn(move |cancel| {
                            backend.diff(
                                &repo,
                                &DiffOptions {
                                    target: DiffTarget::Commit { id },
                                    ..Default::default()
                                },
                                &cancel,
                            )
                        }) {
                            Ok(task) => self.tab.diff = Some(task),
                            Err(e) => self.error = Some(user_error(&e)),
                        }
                    }
                }
                Err(Error::Cancelled) => {
                    self.notice = Some("Read cancelled. Retry to load this revision.".into())
                }
                Err(e) => self.error = Some(user_error(&e)),
            }
        }
        if let Some(result) = self.tab.diff.as_mut().and_then(ReadTask::try_finish) {
            self.tab.diff = None;
            match result {
                Ok(mut p) => {
                    file_history::readable_headers(&mut p);
                    self.tab.patch = Some(p);
                }
                Err(Error::Cancelled) => {
                    self.notice = Some("Diff cancelled. Retry to reload.".into())
                }
                Err(e) => self.error = Some(user_error(&e)),
            }
        }
        file_history::poll_tab(&mut self.tab, &self.backend, ctx);
        self.tab.diff_tools.poll();
        self.tab.image.poll(ctx);
        if !self.running()
            && self.notice.as_deref() == Some("Stopping read. Previous content stays available.")
        {
            self.notice = Some("Read stopped. Retry to reload.".into());
        }
        if self.running() {
            ctx.request_repaint_after(Duration::from_millis(30));
        }
    }
    fn ui(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        self.size = [ui.available_width(), ui.available_height()];
        self.widgets.clear();
        for event in ctx.input(|i| i.events.clone()) {
            if matches!(event, Event::Screenshot { .. }) {
                if self.screenshots.len() == 4 {
                    self.screenshots.remove(0);
                }
                self.screenshots.push(event);
            }
        }
        if ctx.input(|i| i.viewport().close_requested()) {
            self.close();
            ctx.request_repaint_of(ViewportId::ROOT);
            return;
        }
        self.poll(&ctx);
        ui.painter().rect_filled(ui.max_rect(), 0.0, BG());
        let mut intents = Vec::new();
        let running = self.running();
        // The window frame's × closes a review, so there is no Close button here.
        let short = self.request.commit.as_str()[..8].to_owned();
        let commit = self.tab.loaded_commits().first().map(|c| {
            let message = String::from_utf8_lossy(&c.message);
            (
                c.subject_lossy().into_owned(),
                format!(
                    "{} · {}",
                    String::from_utf8_lossy(&c.author.name),
                    date(c.author.timestamp, c.author.offset_minutes)
                ),
                message
                    .split_once('\n')
                    .map(|(_, body)| body.trim().to_owned())
                    .filter(|body| !body.is_empty()),
            )
        });
        let file = self.request.path.as_ref().map(|p| p.display_lossy().into_owned());
        let kind = if self.request.blame { "Blame" } else { "History" };
        let (image, title, description) = match (&file, &commit) {
            (Some(path), Some((subject, _, _))) => (Icon::File, path.clone(), format!("{kind} at {short} · {subject}")),
            (Some(path), None) => (Icon::File, path.clone(), format!("{kind} at {short}")),
            (None, Some((subject, meta, _))) => (Icon::Commit, subject.clone(), meta.clone()),
            (None, None) => (Icon::Commit, format!("Commit {short}"), self.tab.title.clone()),
        };
        let body = commit.and_then(|(_, _, body)| body).filter(|_| file.is_none());
        Panel::top(Id::new(("review-header", self.id)))
            .frame(Frame::new().fill(BG()))
            .show(ui, |ui| {
                pages::header(ui, image, &title, &description, |ui| {
                    let r = ui
                        .add(Button::new(RichText::new(&short).monospace()))
                        .on_hover_text("Copy commit ID");
                    insert(&mut self.widgets, "review-copy-id", &r, ui);
                    if r.clicked() {
                        ui.ctx().copy_text(self.request.commit.to_string());
                    }
                    pages::pill(ui, "Pinned · read only", MUTED());
                    // Text only: an animated child window can starve native redraws.
                    if running {
                        if button(ui, &mut self.widgets, "review-cancel", "Stop reading", true) {
                            self.cancel();
                        }
                        ui.label(RichText::new("Reading").size(12.0).color(MUTED()));
                    } else if (self.error.is_some() || self.notice.is_some())
                        && button(ui, &mut self.widgets, "review-retry", "Retry", true)
                    {
                        self.load();
                    }
                });
                if body.is_some() || self.error.is_some() || self.notice.is_some() {
                    Frame::new()
                        .inner_margin(Margin { left: 24, right: 20, top: 0, bottom: 12 })
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.spacing_mut().item_spacing.y = 8.0;
                            if let Some(body) = &body {
                                ScrollArea::vertical()
                                    .id_salt(("review-body", self.id))
                                    .max_height(90.0)
                                    .show(ui, |ui| {
                                        ui.add(Label::new(RichText::new(body).color(MUTED())).wrap());
                                    });
                            }
                            if let Some(error) = &self.error {
                                pages::callout(ui, RED(), error);
                            }
                            if let Some(notice) = &self.notice {
                                pages::note(ui, notice);
                            }
                        });
                }
            });
        if self.request.path.is_some() {
            CentralPanel::default()
                .frame(Frame::new().fill(BG()))
                .show(ui, |ui| {
                    file_history::view(
                        ui,
                        &mut self.tab,
                        &mut intents,
                        &mut self.widgets,
                        &[],
                        false,
                    )
                });
        } else {
            self.commit_ui(ui, &mut intents);
        }
        for intent in intents {
            if let Intent::File(action) = intent {
                if matches!(action, FileIntent::Load) {
                    if let Ok(path) =
                        RepoPath::new(self.tab.file_history.path_input.as_bytes().to_vec())
                    {
                        self.tab.file_history.load(
                            self.backend.clone(),
                            self.repository.clone(),
                            Some((
                                self.request.commit.clone(),
                                path,
                                self.tab.file_history.directory_input,
                            )),
                        );
                    } else {
                        self.tab.file_history.error =
                            Some("Enter a repository-relative file path.".into());
                    }
                } else {
                    file_history::intent(&mut self.tab, self.backend.clone(), action);
                }
            }
        }
    }
    fn commit_ui(&mut self, ui: &mut Ui, _intents: &mut Vec<Intent>) {
        let wide = ui.available_width() >= 760.0;
        if wide && let Some(patch) = &self.tab.patch {
            let files: Vec<_> = patch.files.iter().map(|f| (f.label.clone(), f.added, f.removed)).collect();
            Panel::left(Id::new(("review-file-list", self.id)))
                .default_size(260.0)
                .size_range(200.0..=(ui.available_width() * 0.45))
                .resizable(true)
                .frame(crate::style::frame())
                .show(ui, |ui| {
                    Frame::new().inner_margin(Margin::symmetric(14, 10)).show(ui, |ui| {
                        pages::caption(ui, &format!("Files changed · {}", files.len()));
                    });
                    ScrollArea::vertical().id_salt(("review-files-scroll", self.id)).show(ui, |ui| {
                        Frame::new().inner_margin(Margin { left: 6, right: 6, top: 0, bottom: 10 }).show(ui, |ui| {
                            ui.spacing_mut().item_spacing.y = 1.0;
                            for (i, (label, added, removed)) in files.iter().enumerate() {
                                let r = file_row(ui, label, *added, *removed, i == self.tab.patch_file);
                                insert(&mut self.widgets, &format!("review-file-{i}"), &r, ui);
                                if r.clicked() {
                                    self.tab.patch_file = i;
                                }
                            }
                        });
                    });
                });
        }
        if !wide {
            Panel::top(Id::new(("review-commit", self.id)))
                .frame(Frame::new().fill(BG()).inner_margin(Margin::symmetric(20, 8)))
                .show(ui, |ui| {
                if let Some(patch) = &self.tab.patch {
                    let selector = ComboBox::from_id_salt(("review-files", self.id))
                        .width((ui.available_width() - 20.0).max(160.0))
                        .selected_text(
                            patch
                                .files
                                .get(self.tab.patch_file)
                                .map_or("No changed files", |f| f.label.as_str()),
                        )
                        .show_ui(ui, |ui| {
                            for (i, file) in patch.files.iter().enumerate() {
                                let row =
                                    ui.selectable_value(&mut self.tab.patch_file, i, &file.label);
                                self.widgets
                                    .insert(format!("review-file-{i}"), (&row).into());
                            }
                        });
                    self.widgets
                        .insert("review-files".into(), (&selector.response).into());
                }
            });
        }
        CentralPanel::default()
            .frame(Frame::new().fill(BG()))
            .show(ui, |ui| {
                // Only read intents exist here. Patch editing/staging is disabled.
                if matches!(
                    crate::patch_view::patch_view(
                        ui,
                        &mut self.tab,
                        false,
                        false,
                        &mut self.widgets
                    ),
                    Some(Intent::ReloadImage)
                ) {
                    self.load();
                }
            });
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
    insert(widgets, key, &r, ui);
    r.clicked()
}
/// Publishes a control by its visible part only.
fn insert(widgets: &mut HashMap<String, UiControl>, key: &str, r: &Response, ui: &Ui) {
    let rect = r.interact_rect.intersect(ui.clip_rect());
    if rect.is_positive() {
        widgets.insert(
            key.into(),
            UiControl {
                rect,
                enabled: r.enabled(),
            },
        );
    }
}
/// One changed file: its name, the folder after it, and the line counts.
fn file_row(ui: &mut Ui, label: &str, added: usize, removed: usize, selected: bool) -> Response {
    let response = ui
        .add(Button::new("").frame(false).min_size(vec2(ui.available_width(), 30.0)))
        .on_hover_text(label);
    response.widget_info(|| WidgetInfo::selected(WidgetType::Button, true, selected, label));
    let r = response.rect;
    let painter = ui.painter();
    row_background(painter, r, selected, response.hovered());
    let (dir, name) = label.rsplit_once('/').unwrap_or(("", label));
    let minus = format!("−{removed}");
    let minus_width = painter.layout_no_wrap(minus.clone(), FontId::monospace(11.5), RED()).size().x;
    let plus = format!("+{added}");
    let plus_width = painter.layout_no_wrap(plus.clone(), FontId::monospace(11.5), GREEN()).size().x;
    painter.text(pos2(r.right() - 10.0, r.center().y), Align2::RIGHT_CENTER, minus, FontId::monospace(11.5), RED());
    painter.text(pos2(r.right() - 16.0 - minus_width, r.center().y), Align2::RIGHT_CENTER, plus, FontId::monospace(11.5), GREEN());
    let right = r.right() - 26.0 - minus_width - plus_width;
    let clip = painter.with_clip_rect(Rect::from_min_max(r.min, pos2(right, r.bottom())));
    let name = painter.layout_no_wrap(name.into(), FontId::proportional(13.0), TEXT());
    let name_width = name.size().x;
    clip.galley(pos2(r.left() + 10.0, r.center().y - name.size().y / 2.0), name, TEXT());
    if !dir.is_empty() {
        clip.text(pos2(r.left() + 18.0 + name_width, r.center().y), Align2::LEFT_CENTER, dir, FontId::proportional(12.0), FAINT());
    }
    response
}
impl Rocket {
    pub(crate) fn open_review(&mut self, request: Request) {
        self.open_review_for(self.active, request);
    }
    pub(crate) fn open_review_for(&mut self, index: usize, request: Request) {
        if self.reviews.len() >= 8 {
            self.notice = Some("Close a review before opening another (maximum eight).".into());
            return;
        }
        if let (Some(backend), Some(tab)) = (self.backend.clone(), self.tabs.get(index))
            && let Some(snapshot) = &tab.snapshot {
                self.reviews.push(Arc::new(Mutex::new(Review::new(
                    tab.path.clone(),
                    snapshot.repository.clone(),
                    backend,
                    request,
                ))));
            }
    }
    pub(crate) fn review_windows(&mut self, ctx: &Context) {
        self.reviews
            .retain(|window| !window.lock().unwrap_or_else(|e| e.into_inner()).closed);
        for window in &self.reviews {
            let (id, title) = {
                let mut r = window.lock().unwrap_or_else(|e| e.into_inner());
                if self.quitting {
                    r.close();
                    continue;
                }
                r.poll(ctx);
                (
                    r.viewport(),
                    format!(
                        "Rocket review · {} · {}",
                        r.tab.title,
                        &r.request.commit.as_str()[..8]
                    ),
                )
            };
            let window = window.clone();
            ctx.show_viewport_deferred(
                id,
                ViewportBuilder::default()
                    .with_title(title)
                    .with_icon(crate::app_icon::icon())
                    .with_active(!self.background_reviews)
                    .with_inner_size([1100.0, 760.0])
                    .with_min_inner_size([620.0, 440.0]),
                move |ui, _| {
                    window.lock().unwrap_or_else(|e| e.into_inner()).ui(ui);
                },
            );
        }
    }
}

/// Explicit native launch contract used by rocket-cli; ordinary path launches stay unchanged.
pub fn launch_args(args: &[std::ffi::OsString]) -> Result<Option<(PathBuf, Request)>> {
    if args.first().is_none_or(|a| a != "--review") {
        return Ok(None);
    }
    if args.len() != 6 && args.len() != 8 {
        return Err(Error::InvalidInput(
            "Invalid native review arguments.".into(),
        ));
    }
    if args[2] != "--repo" || args[4] != "--commit" {
        return Err(Error::InvalidInput(
            "Invalid native review arguments.".into(),
        ));
    }
    let mode = args[1].to_str().unwrap_or("");
    if !matches!(mode, "commit" | "file" | "blame")
        || (mode == "commit") != (args.len() == 6)
        || (args.len() == 8 && args[6] != "--path")
    {
        return Err(Error::InvalidInput(
            "Invalid native review mode/path.".into(),
        ));
    }
    let commit = ObjectId::parse(args[5].to_str().unwrap_or(""))?;
    let path = if args.len() == 8 {
        Some(RepoPath::new(
            args[7]
                .to_str()
                .ok_or_else(|| Error::InvalidInput("Review path must be Unicode.".into()))?
                .as_bytes()
                .to_vec(),
        )?)
    } else {
        None
    };
    Ok(Some((
        PathBuf::from(&args[3]),
        Request {
            commit,
            path,
            blame: mode == "blame",
        },
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn independent_windows_finish_image_previews_and_release_on_close() {
        use crate::{batch::flow_tests::open, test_common as common};
        use std::{thread, time::Instant};
        common::isolated(
            "ROCKET_REVIEW_UI_CASE",
            "reviews::tests::independent_windows_finish_image_previews_and_release_on_close",
            || {
                let f = common::Fixture::new();
                for (label, color) in [("before", [30, 80, 120, 255]), ("after", [90, 120, 150, 255])] {
                    image::RgbaImage::from_pixel(96, 64, image::Rgba(color))
                        .save(f.repository.join("image.png")).unwrap();
                    f.commit(label);
                }
                let commit = ObjectId::parse(std::str::from_utf8(&f.run(&["rev-parse", "HEAD"])).unwrap().trim()).unwrap();
                let ctx = Context::default();
                let app = open(&ctx, &f);
                // Each commit window owns a separate Tab and must consume both
                // ordinary decoding and pixel analysis without main-tab polling.
                for mode in [crate::image_view::Mode::Side, crate::image_view::Mode::Pixels] {
                    let mut review = Review::new(f.repository.clone(), f.repo(), app.backend.clone().unwrap(), Request {
                        commit: commit.clone(), path: None, blame: false,
                    });
                    review.tab.image.mode = mode;
                    let deadline = Instant::now() + Duration::from_secs(30);
                    loop {
                        let mut output = ctx.run_ui(RawInput {
                            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1480.0, 920.0))),
                            ..Default::default()
                        }, |ui| review.ui(ui));
                        output.textures_delta.clear();
                        assert!(review.error.is_none(), "{:?}", review.error);
                        assert!(review.tab.image.error.is_none(), "{:?}", review.tab.image.error);
                        if review.tab.image.loaded && !review.running() { break; }
                        assert!(Instant::now() < deadline, "independent window did not consume the {mode:?} image result");
                        thread::sleep(Duration::from_millis(10));
                    }
                    assert_eq!(review.tab.image.after.as_ref().unwrap().width, 96);
                    assert_eq!(review.tab.image.after.as_ref().unwrap().height, 64);
                    if mode == crate::image_view::Mode::Pixels {
                        assert!(review.tab.image.stats.is_some());
                        assert!(review.tab.image.difference_error.is_none());
                    }
                    review.close();
                    review.poll(&ctx);
                    assert!(!review.running());
                    assert!(!review.tab.image.loaded);
                    assert!(review.tab.image.before.is_none() && review.tab.image.after.is_none());
                }
            },
        );
    }

    #[test]
    fn review_windows_open_active_unless_mcp_asks_for_background() {
        use crate::{batch::flow_tests::open, test_common as common};
        common::isolated(
            "ROCKET_REVIEW_ACTIVE_CASE",
            "reviews::tests::review_windows_open_active_unless_mcp_asks_for_background",
            || {
                let f = common::Fixture::new();
                f.write("reviewed.txt", "reviewed\n");
                f.commit("reviewed");
                let commit = ObjectId::parse(std::str::from_utf8(&f.run(&["rev-parse", "HEAD"])).unwrap().trim()).unwrap();
                let ctx = Context::default();
                // Otherwise egui draws the window inside its parent and creates no viewport.
                ctx.set_embed_viewports(false);
                let mut app = open(&ctx, &f);
                let review = Review::new(f.repository.clone(), f.repo(), app.backend.clone().unwrap(), Request {
                    commit, path: None, blame: false,
                });
                let id = review.viewport();
                app.reviews.push(Arc::new(Mutex::new(review)));
                let active = |app: &mut crate::app::Rocket| {
                    ctx.run_ui(RawInput::default(), |ui| app.review_windows(ui.ctx())).viewport_output[&id].builder.active
                };
                assert_eq!(active(&mut app), Some(true));
                app.background_reviews = true;
                assert_eq!(active(&mut app), Some(false));
                app.reviews[0].lock().unwrap().close();
            },
        );
    }

    #[test]
    fn launch_contract_keeps_paths_literal_and_rejects_bad_revisions() {
        let args = |a: &[&str]| a.iter().map(std::ffi::OsString::from).collect::<Vec<_>>();
        let oid = "1111111111111111111111111111111111111111";
        let r = launch_args(&args(&[
            "--review",
            "blame",
            "--repo",
            "D:/repos/資料",
            "--commit",
            oid,
            "--path",
            "src/表示.rs",
        ]))
        .unwrap()
        .unwrap();
        assert!(r.1.blame);
        assert_eq!(r.1.path.unwrap().as_bytes(), "src/表示.rs".as_bytes());
        assert!(
            launch_args(&args(&[
                "--review", "commit", "--repo", "x", "--commit", "--help"
            ]))
            .is_err()
        );
        assert!(
            launch_args(&args(&[
                "--review",
                "file",
                "--repo",
                "x",
                "--commit",
                oid,
                "--path",
                "../escape"
            ]))
            .is_err()
        );
        assert!(launch_args(&args(&["C:/normal repo"])).unwrap().is_none());
    }
}
