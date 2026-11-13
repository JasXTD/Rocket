//! Locally derived browser links, with an explicit choice for ambiguous remotes.
use crate::{
    app::Rocket,
    backend::{Backend, user_error},
};
use eframe::egui::*;
use rocket_git::*;
pub struct State {
    pub commit: ObjectId,
    pub links: Vec<CommitBrowserLink>,
    pub selected: Option<usize>,
    pub error: Option<String>,
    pub notice: Option<String>,
    pub custom: bool,
    pub provider: ForgeProvider,
    pub server: String,
    pub remote: String,
    options_open: bool,
    repo: Repository,
    task: Option<ReadTask<Vec<CommitBrowserLink>>>,
}
impl State {
    pub fn running(&self) -> bool {
        self.task.is_some()
    }
    fn load(&mut self, backend: Backend) {
        self.task = None;
        self.links.clear();
        self.selected = None;
        self.error = None;
        self.notice = None;
        let config = if self.custom {
            match ForgeConfig::new(self.provider, &self.server) {
                Ok(c) => Some(c),
                Err(e) => {
                    self.error = Some(user_error(&e));
                    return;
                }
            }
        } else {
            None
        };
        let repo = self.repo.clone();
        let id = self.commit.clone();
        let remote = self.remote.trim().to_owned();
        match ReadTask::spawn(move |cancel| {
            backend.log.request("commit-link", |log| {
                repo.with_log(log.clone()).commit_browser_links(
                    &id,
                    (!remote.is_empty()).then_some(remote.as_bytes()),
                    config,
                    &cancel,
                )
            })
        }) {
            Ok(t) => self.task = Some(t),
            Err(e) => self.error = Some(user_error(&e)),
        }
    }
}
impl Rocket {
    pub(crate) fn open_commit_link(&mut self, commit: ObjectId) {
        if let (Some(backend), Some(tab)) = (self.backend.clone(), self.tabs.get(self.active))
            && let Some(snapshot) = &tab.snapshot
        {
            let mut state = State {
                commit,
                links: vec![],
                selected: None,
                error: None,
                notice: None,
                custom: false,
                provider: ForgeProvider::GitLab,
                server: String::new(),
                remote: String::new(),
                options_open: false,
                repo: snapshot.repository.clone(),
                task: None,
            };
            state.load(backend);
            self.commit_link = Some(state);
        }
    }
    pub(crate) fn commit_link_ui(&mut self, ctx: &Context) {
        let Some(mut s) = self.commit_link.take() else {
            return;
        };
        if let Some(result) = s.task.as_mut().and_then(ReadTask::try_finish) {
            s.task = None;
            match result {
                Ok(links) => {
                    s.selected = (links.len() == 1).then_some(0);
                    s.links = links;
                    if s.links.is_empty() {
                        s.options_open = true;
                    }
                }
                Err(Error::Cancelled) => s.notice = Some("Link read cancelled.".into()),
                Err(e) => s.error = Some(user_error(&e)),
            }
        }
        let mut close = false;
        let mut refresh = false;
        let mut open = None;
        let mut copied = false;
        Modal::new(Id::unique("commit-link-review"))
            .frame(crate::pages::modal_frame())
            .show(ctx, |ui| {
            use crate::{pages, style::*};
            let widgets = &mut self.frame_widgets;
            ui.set_width((ctx.content_rect().width() - 48.0).clamp(280.0, 580.0));
            ui.spacing_mut().item_spacing.y = 0.0;
            Frame::new().inner_margin(Margin { left: 24, right: 20, top: 22, bottom: 18 }).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal_top(|ui| {
                    let width = (ui.available_width() - 40.0).max(160.0);
                    ui.allocate_ui_with_layout(vec2(width, 40.0), Layout::left_to_right(Align::Min), |ui| {
                        ui.spacing_mut().item_spacing.x = 12.0;
                        let (tile, _) = ui.allocate_exact_size(vec2(38.0, 38.0), Sense::hover());
                        ui.painter().rect_filled(tile, 10.0, ACCENT_SOFT());
                        icon(ui.painter(), tile.shrink(10.0), Icon::Link, BLUE());
                        ui.vertical(|ui| {
                            ui.set_width((width - 50.0).max(110.0));
                            ui.spacing_mut().item_spacing.y = 3.0;
                            ui.label(RichText::new("Commit link").font(semibold(18.0)));
                            pages::note(ui, "Copy a link or open this commit on your host.");
                        });
                    });
                    let x = crate::chrome::icon_button(ui, Icon::Close, true, false, 28.0).on_hover_text("Close (Esc)");
                    widgets.insert("commit-link-close".into(), (&x).into());
                    close |= x.clicked();
                });
            });
            // Only the body scrolls: Close, Copy and Open remain reachable on short windows.
            let body = ScrollArea::vertical()
                .id_salt("commit-link-body")
                .max_height((ctx.content_rect().height() - 220.0).clamp(120.0, 660.0))
                .auto_shrink([false, true])
                .show(ui, |ui| {
            Frame::new().inner_margin(Margin { left: 24, right: 24, top: 0, bottom: 18 }).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 12.0;
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Commit").size(12.0).color(MUTED()));
                    let id = s.commit.as_str();
                    ui.label(RichText::new(id.get(..12).unwrap_or(id)).monospace().size(12.0).background_color(RAISED()))
                        .on_hover_text(id);
                });
                if s.running() {
                    pages::well(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label(RichText::new("Reading repository remotes").size(12.5).color(MUTED()));
                            let r = pages::ghost(ui, true, "Stop");
                            pages::control(widgets, "commit-link-cancel", &r, ui.clip_rect());
                            if r.clicked() {
                                s.task = None;
                                s.notice = Some("Link read cancelled. Refresh links to try again.".into());
                            }
                        });
                    });
                } else if !s.links.is_empty() {
                    pages::caption(ui, if s.links.len() == 1 { "Remote" } else { "Choose a remote" });
                    ui.scope(|ui| {
                        ui.spacing_mut().item_spacing.y = 6.0;
                        for (i, link) in s.links.iter().enumerate() {
                            let selected = s.selected == Some(i);
                            let remote = String::from_utf8_lossy(&link.remote);
                            let address = destination(link, &s.commit);
                            let r = Frame::new().fill(BG())
                                .stroke(Stroke::new(1.0, if selected { BLUE() } else { HAIRLINE() }))
                                .corner_radius(8).inner_margin(4).show(ui, |ui| {
                                    pages::row(ui, pages::Row {
                                        image: Icon::Remote, title: &remote, detail: address,
                                        pills: vec![(link.provider.label().into(), MUTED())], indent: 0.0,
                                    }, selected, true)
                                }).inner.on_hover_text(format!("{remote}\n{}", link.url));
                            pages::control(widgets, format!("commit-link-remote-{i}"), &r, ui.clip_rect());
                            if r.clicked() {
                                s.selected = Some(i);
                                s.notice = None;
                            }
                        }
                    });
                } else {
                    pages::well(ui, |ui| {
                        ui.label(RichText::new("No link available").font(semibold(13.0)));
                        pages::note(ui, "Choose a hosted remote or configure a custom server in Link options.");
                    });
                }
                pages::well(ui, |ui| {
                    pages::caption(ui, "Link preview");
                    if let Some(link) = s.selected.and_then(|i| s.links.get(i)) {
                        ui.add(Label::new(RichText::new(&link.url).monospace().size(12.0)).wrap());
                    } else {
                        pages::note(ui, if s.running() { "The link will appear when the read finishes." }
                            else if s.links.is_empty() { "Refresh links after choosing your remote settings." }
                            else { "Select a remote above to preview its link." });
                    }
                    pages::note(ui, "The link uses this exact commit. It may not be on the remote yet.");
                });
                if let Some(error) = &s.error {
                    pages::callout(ui, RED(), error);
                }
                if let Some(notice) = &s.notice {
                    ui.add(Label::new(RichText::new(notice).size(12.0)
                        .color(if notice.starts_with("Link copied") { GREEN() } else { MUTED() })).wrap());
                }
                ui.separator();
                let toggle = pages::tool(ui, if s.options_open { Icon::Chevron } else { Icon::ChevronRight }, "Link options", true);
                pages::control(widgets, "commit-link-options", &toggle, ui.clip_rect());
                if toggle.clicked() { s.options_open = !s.options_open; }
                let mut changed = false;
                if s.options_open {
                    ui.label(RichText::new("Remote name").size(12.0).color(MUTED()));
                    let r = ui.add(TextEdit::singleline(&mut s.remote).hint_text("All remotes").desired_width(f32::INFINITY));
                    pages::control(widgets, "commit-link-name", &r, ui.clip_rect());
                    changed |= r.changed();
                    let custom = ui.add(checkbox(&mut s.custom, "Use a custom hosting server"));
                    pages::control(widgets, "commit-link-custom", &custom, ui.clip_rect());
                    changed |= custom.changed();
                    if s.custom {
                        ui.label(RichText::new("Provider").size(12.0).color(MUTED()));
                        let before = s.provider;
                        let provider = ComboBox::from_id_salt("commit-link-provider").width(ui.available_width()).selected_text(s.provider.label()).show_ui(ui, |ui| {
                            for p in ForgeProvider::ALL {
                                let r = ui.selectable_value(&mut s.provider, p, p.label());
                                pages::control(widgets, format!("commit-link-provider-{}", p.key()), &r, ui.clip_rect());
                            }
                        });
                        pages::control(widgets, "commit-link-provider", &provider.response, ui.clip_rect());
                        changed |= before != s.provider;
                        ui.label(RichText::new("Server address").size(12.0).color(MUTED()));
                        let server = ui.add(TextEdit::singleline(&mut s.server).hint_text("https://git.example.com").desired_width(f32::INFINITY));
                        pages::control(widgets, "commit-link-server", &server, ui.clip_rect());
                        changed |= server.changed();
                        pages::note(ui, "Use the HTTPS server address, including any context path. No sign-in is needed to create a link.");
                    }
                    let r = pages::tool(ui, Icon::Refresh, "Refresh links", !s.running());
                    pages::control(widgets, "commit-link-refresh", &r, ui.clip_rect());
                    refresh = r.clicked();
                }
                if changed {
                    s.task = None;
                    s.links.clear();
                    s.selected = None;
                    s.error = None;
                    s.notice = Some("Settings changed. Refresh links to update the preview.".into());
                }
            });
            });
            // egui reports the pre-shrink inner rect. Clamp the automation target
            // to the space actually allocated, so it cannot include the footer.
            let body_rect = body.inner_rect.intersect(Rect::from_min_max(
                body.inner_rect.min, pos2(body.inner_rect.right(), ui.cursor().top()),
            ));
            pages::region(widgets, "commit-link-scroll", body_rect, true);
            let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
            ui.painter().hline(rect.x_range(), rect.center().y, Stroke::new(1.0, HAIRLINE()));
            Frame::new().fill(RAISED())
                .corner_radius(CornerRadius { nw: 0, ne: 0, sw: 12, se: 12 })
                .inner_margin(Margin { left: 24, right: 24, top: 14, bottom: 18 }).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().interact_size.y = 34.0;
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    let link = (!s.running()).then(|| s.selected.and_then(|i| s.links.get(i))).flatten();
                    let browse = pages::primary(ui, link.is_some(), "Open in browser");
                    widgets.insert("commit-link-open".into(), (&browse).into());
                    if browse.clicked() { open = link.map(|l| l.url.clone()); }
                    let copy = ui.add_enabled(link.is_some(), Button::new("Copy link"));
                    widgets.insert("commit-link-copy".into(), (&copy).into());
                    if copy.clicked() && let Some(link) = link {
                        ctx.copy_text(link.url.clone());
                        copied = true;
                    }
                });
            });
        });
        if copied {
            s.notice = Some("Link copied.".into());
        }
        if let Some(url) = open {
            // Only URLs built from ForgeConfig + validated path + ObjectId reach here.
            if let Some(opened) = &mut self.opened_urls {
                opened.push(url);
            } else {
                ctx.open_url(OpenUrl::new_tab(url));
            }
            if let Some(backend) = &self.backend {
                let _ = backend.log.request("commit-link-open", |_| Ok(()));
            }
        }
        if refresh && let Some(backend) = self.backend.clone() {
            s.load(backend);
        }
        if ctx.input(|i| i.key_pressed(Key::Escape)) {
            close = true;
        }
        if s.running() {
            ctx.request_repaint_after(std::time::Duration::from_millis(30));
        }
        if !close {
            self.commit_link = Some(s);
        }
    }
}

/// Presentation only: remove the exact validated commit suffix, never a repository
/// segment that happens to contain the word "commit". Copy/Open use the original URL.
fn destination<'a>(link: &'a CommitBrowserLink, commit: &ObjectId) -> &'a str {
    let suffix = match link.provider {
        ForgeProvider::GitLab => "/-/commit/",
        ForgeProvider::Bitbucket | ForgeProvider::BitbucketServer => "/commits/",
        _ => "/commit/",
    };
    let base = link
        .url
        .strip_suffix(commit.as_str())
        .and_then(|url| url.strip_suffix(suffix))
        .unwrap_or(&link.url);
    base.strip_prefix("https://").unwrap_or(base)
}
