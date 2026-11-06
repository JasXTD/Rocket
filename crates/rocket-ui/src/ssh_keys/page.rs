//! Preferences → SSH keys: every key listed once beside the chosen key's location,
//! fingerprint and public key, with New key and Add key file, and the open repository's
//! use of a key. A passphrase question, a repository review or a key form takes the
//! right side while it is open.
use super::*;

/// Where a listed key comes from: a profile Rocket made or set up, or the scan of the
/// SSH folder. A found key that has a profile is listed once, as the profile.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Source {
    Profile(usize),
    Found(usize),
}
impl Source {
    /// The row's automation name: profile rows keep the names they had.
    fn id(self) -> String {
        match self {
            Source::Profile(i) => format!("ssh-profile-{i}"),
            Source::Found(i) => format!("ssh-found-{i}"),
        }
    }
    /// The automation name of the row's tick while the row isn't the chosen one.
    fn tick_id(self) -> String {
        match self {
            Source::Profile(i) => format!("ssh-tick-profile-{i}"),
            Source::Found(i) => format!("ssh-tick-found-{i}"),
        }
    }
}

/// A key as the page lists it.
struct Listed {
    source: Source,
    name: String,
    /// Its kind and place, such as "Ed25519 · in .ssh".
    detail: String,
    path: PathBuf,
    fingerprint: String,
    public_key: String,
    kind: String,
    /// Known for a profile. A scan never reads a private key, so not for a found key.
    encrypted: Option<bool>,
}

impl State {
    /// Every key once: the SSH folder's in name order, then Rocket's others.
    fn listed(&self) -> Vec<Listed> {
        let folder = format!("in {}", file_name(&self.scan_directory));
        let mut taken = vec![false; self.profiles.len()];
        let mut keys: Vec<Listed> = Vec::new();
        for (i, k) in self.discovery.iter().flat_map(|d| &d.keys).enumerate() {
            let profile = (0..self.profiles.len())
                .find(|&j| !taken[j] && same_file(&self.profiles[j].key, &k.key));
            if let Some(j) = profile {
                taken[j] = true;
                keys.push(self.profile_entry(j, file_name(&k.key), &folder));
                continue;
            }
            let kind = kind_label(&k.kind);
            keys.push(Listed {
                source: Source::Found(i),
                name: file_name(&k.key),
                detail: format!("{kind} · {folder}"),
                path: k.key.clone(),
                fingerprint: k.fingerprint.clone(),
                public_key: k.public_key().to_owned(),
                kind,
                encrypted: None,
            });
        }
        for (j, p) in self.profiles.iter().enumerate() {
            if taken[j] {
                continue;
            }
            let entry = if under(&p.key, &self.root) {
                self.profile_entry(j, p.alias.clone(), "made by Rocket")
            } else {
                // A second profile for a listed key is told apart by its alias.
                let name = file_name(&p.key);
                let name = if keys.iter().any(|k| k.name == name) { p.alias.clone() } else { name };
                let place = format!("in {}", p.key.parent().map(file_name).unwrap_or_default());
                self.profile_entry(j, name, &place)
            };
            keys.push(entry);
        }
        keys
    }

    fn profile_entry(&self, j: usize, name: String, place: &str) -> Listed {
        let p = &self.profiles[j];
        let kind = kind_label(p.public_key.split_whitespace().next().unwrap_or_default());
        let passphrase = if p.encrypted { " · passphrase" } else { "" };
        Listed {
            source: Source::Profile(j),
            name,
            detail: format!("{kind} · {place}{passphrase}"),
            path: p.key.clone(),
            fingerprint: p.fingerprint.clone(),
            public_key: p.public_key.trim().to_owned(),
            kind,
            encrypted: Some(p.encrypted),
        }
    }

    pub(super) fn source(&self) -> Option<Source> {
        self.selected.map(Source::Profile).or(self.found.map(Source::Found))
    }

    /// Choose a key. A form or review that was open for another key closes.
    pub(super) fn select(&mut self, source: Source) {
        let chosen = match source {
            Source::Profile(i) => (Some(i), None),
            Source::Found(i) => (None, Some(i)),
        };
        if chosen != (self.selected, self.found) {
            self.preview = None;
            self.creating = false;
            self.discovered_choice = None;
            self.review_after_create = false;
            self.authentication = SshAuthentication::Agent;
        }
        (self.selected, self.found) = chosen;
    }
}

/// What the page asked for while it was drawn, done once drawing ends.
#[derive(Default)]
struct Out {
    action: Option<&'static str>,
    pick: Option<Source>,
    /// A key's tick: unlock it, or remove it from ssh-agent when it is unlocked.
    tick: Option<(Source, bool)>,
    form: Option<Form>,
    copy: Option<(String, &'static str)>,
    reveal: Option<PathBuf>,
}
enum Form {
    New,
    File,
    /// A found key for the open repository: its settings first, then the review.
    Use(usize),
}

impl Rocket {
    pub(crate) fn ssh_preferences(&mut self, ui: &mut Ui) {
        let busy = self.ssh.running();
        let repository = self.tab().map(|t| t.title.clone());
        let endpoint = self
            .tab()
            .and_then(|t| t.snapshot.as_ref())
            .and_then(|s| remote_endpoint(&s.remotes));
        let s = &mut self.ssh;
        let widgets = &mut self.frame_widgets;
        let mut out = Out::default();
        ui.spacing_mut().item_spacing.y = 10.0;
        if !s.enabled {
            messages(ui, s);
            pages::callout(ui, RED(), "SSH management requires a marked disposable fixture in this MCP window.");
            return;
        }
        let keys = s.listed();
        if !keys.iter().any(|k| Some(k.source) == s.source()) {
            // The first key when none is chosen or the chosen one has gone. Unlike a click,
            // this keeps an open form: a rescan must not throw away what was typed.
            (s.selected, s.found) = match keys.first().map(|k| k.source) {
                Some(Source::Profile(i)) => (Some(i), None),
                Some(Source::Found(i)) => (None, Some(i)),
                None => (None, None),
            };
        }
        status_row(ui, s, widgets, busy, &mut out);
        messages(ui, s);
        let open = s.creating || s.prompt.is_some() || s.preview.is_some();
        if keys.is_empty() && !open {
            if s.loaded {
                empty_state(ui, s, widgets, &repository, busy, &mut out);
            } else if busy {
                pages::loading(ui, "Looking for SSH keys");
            }
        } else if ui.available_width() >= 620.0 {
            // The list beside the chosen key, together as tall as the page.
            let height = (ui.available_height() - 2.0).max(430.0);
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 16.0;
                ui.allocate_ui_with_layout(vec2(262.0, height), Layout::top_down(Align::Min), |ui| {
                    list_card(ui, s, widgets, &keys, busy, height, &mut out);
                });
                let width = ui.available_width();
                ui.allocate_ui_with_layout(vec2(width, height), Layout::top_down(Align::Min), |ui| {
                    right_side(ui, s, widgets, &keys, &repository, busy, Some(height), &mut out);
                });
            });
        } else {
            // A narrow window: the list above the chosen key.
            let height = 36.0 + 48.0 + 52.0 * keys.len().clamp(1, 4) as f32 + 4.0;
            list_card(ui, s, widgets, &keys, busy, height, &mut out);
            right_side(ui, s, widgets, &keys, &repository, busy, None, &mut out);
        }
        if let Some(source) = out.pick {
            s.select(source);
        }
        if let Some((source, unlocked)) = out.tick {
            s.select(source);
            out.action = Some(if unlocked { "unload" } else { "unlock" });
        }
        match out.form {
            Some(Form::New | Form::File) => {
                s.use_existing = matches!(out.form, Some(Form::File));
                s.creating = true;
                s.preview = None;
                s.discovered_choice = None;
                s.review_after_create = false;
            }
            Some(Form::Use(i)) => {
                if let Some(key) = s.discovery.as_ref().and_then(|d| d.keys.get(i)).cloned() {
                    s.alias = alias_for(&file_name(&key.key));
                    if let Some((user, host, port)) = endpoint {
                        if let Some(user) = user {
                            s.user = user;
                        }
                        s.host = host;
                        s.port = port.to_string();
                    }
                    s.existing = key.key.display().to_string();
                    s.discovered_choice = Some(key);
                    s.use_existing = true;
                    s.creating = true;
                    s.preview = None;
                    s.review_after_create = true;
                    s.error = None;
                    s.notice = None;
                }
            }
            None => {}
        }
        if let Some((text, notice)) = out.copy {
            ui.ctx().copy_text(text);
            s.notice = Some(notice.into());
            s.error = None;
        }
        if let Some(folder) = out.reveal {
            self.show_folder(&folder);
        }
        if let Some(a) = out.action {
            self.ssh_action(a);
        }
    }
}

/// ssh-agent's state, what is running, and Rescan.
fn status_row(ui: &mut Ui, s: &State, widgets: &mut HashMap<String, UiControl>, busy: bool, out: &mut Out) {
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        // Reserve the action before laying out status text. The busy message
        // gets its own row so compact windows cannot paint it over Rescan.
        let width = (ui.available_width() - 104.0).max(120.0);
        ui.allocate_ui_with_layout(
            vec2(width, 28.0),
            Layout::left_to_right(Align::Center).with_main_wrap(true),
            |ui| {
                if s.loaded {
                    if s.agent_running {
                        let text = match s.loaded_keys.len() {
                            0 => "ssh-agent running".to_owned(),
                            1 => "ssh-agent running · 1 key unlocked".to_owned(),
                            n => format!("ssh-agent running · {n} keys unlocked"),
                        };
                        pages::pill(ui, &text, GREEN());
                    } else {
                        pages::pill(ui, "ssh-agent isn't running", AMBER());
                        ui.label(RichText::new("Direct key mode works without it.").size(12.0).color(HINT()));
                    }
                }
            },
        );
        let r = pages::tool(ui, Icon::Refresh, "Rescan", !busy);
        control(widgets, "ssh-scan", &r, ui.clip_rect());
        if r.clicked() {
            out.action = Some("scan");
        }
    });
    if busy {
        ui.horizontal_wrapped(|ui| {
            ui.add(Spinner::new().size(14.0));
            let text = if s.stopping {
                "Stopping OpenSSH and checking the result"
            } else if matches!(s.operation, "ssh-list" | "ssh-discover") {
                "Looking for keys"
            } else {
                "Working. OpenSSH may ask for a passphrase."
            };
            ui.label(RichText::new(text).size(12.5).color(MUTED()));
        });
    }
}

/// Problems and results, then what the last scan couldn't use.
fn messages(ui: &mut Ui, s: &State) {
    if s.invalid > 0 {
        pages::callout(
            ui,
            AMBER(),
            &format!("{} invalid or incomplete profile folders were left as they are.", s.invalid),
        );
    }
    if let Some(e) = &s.error {
        pages::callout(ui, RED(), e);
    }
    if let Some(e) = &s.discovery_error {
        pages::callout(ui, RED(), e);
    }
    if let Some(n) = &s.notice {
        pages::note(ui, n);
    }
    let Some(d) = &s.discovery else { return };
    let folder = file_name(&d.directory);
    let mut line = |text: String| {
        ui.add(Label::new(RichText::new(text).size(12.0).color(AMBER())).wrap());
    };
    match d.skipped {
        0 => {}
        1 => line(format!("1 file in {folder} was skipped: an incomplete, unreadable or unsupported key pair.")),
        n => line(format!("{n} files in {folder} were skipped: incomplete, unreadable or unsupported key pairs.")),
    }
    if d.truncated {
        line("Scan limit reached. Use Add key file for a key that isn't listed.".into());
    }
}

/// A card: the panel fill and card edge, `height` tall when given.
fn card(ui: &mut Ui, height: Option<f32>, content: impl FnOnce(&mut Ui)) {
    Frame::new()
        .fill(PANEL())
        .stroke(Stroke::new(1.0, CARD_STROKE()))
        .corner_radius(8)
        // A point inside the edge, so rows and strips never cover it.
        .inner_margin(1)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            if let Some(height) = height {
                // The margin and the edge take the other four points.
                ui.set_height(height - 4.0);
            }
            ui.spacing_mut().item_spacing.y = 0.0;
            content(ui);
        });
}

/// A card's header strip: the icon tile, a title and a line under it, and `right`
/// (such as a pill) at its end.
fn card_header(ui: &mut Ui, image: Icon, title: &str, detail: &str, right: impl FnOnce(&mut Ui)) {
    let (bar, _) = ui.allocate_exact_size(vec2(ui.available_width(), 58.0), Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(bar, CornerRadius { nw: 7, ne: 7, sw: 0, se: 0 }, CARD());
    painter.hline(bar.x_range(), bar.bottom() - 0.5, Stroke::new(1.0, CARD_LINE()));
    let tile = Rect::from_min_size(pos2(bar.left() + 16.0, bar.center().y - 17.0), vec2(34.0, 34.0));
    painter.rect(tile, 8.0, RAISED(), Stroke::new(1.0, EDGE_HOT()), StrokeKind::Inside);
    icon(painter, tile.shrink(9.0), image, BLUE());
    let mut end = ui.new_child(
        UiBuilder::new()
            .max_rect(Rect::from_min_max(pos2(tile.right() + 12.0, bar.top()), pos2(bar.right() - 16.0, bar.bottom())))
            .layout(Layout::right_to_left(Align::Center)),
    );
    right(&mut end);
    let used = end.min_rect();
    let right_edge = if used.width() > 0.0 { used.left() - 10.0 } else { bar.right() - 16.0 };
    let left = tile.right() + 12.0;
    let width = (right_edge - left).max(0.0);
    let mut job = text::LayoutJob::simple_singleline(title.to_owned(), semibold(15.0), TEXT());
    job.wrap = text::TextWrapping::truncate_at_width(width);
    let galley = ui.painter().layout_job(job);
    ui.painter().galley(pos2(left, bar.center().y - 1.0 - galley.size().y), galley, TEXT());
    clipped_text(
        ui,
        Rect::from_min_max(pos2(left, bar.center().y + 1.0), pos2(left + width, bar.bottom() - 6.0)),
        detail,
        12.0,
        HINT(),
    );
}

/// The keys, with a tick each for "unlocked in ssh-agent", then New key and Add key file.
fn list_card(
    ui: &mut Ui,
    s: &State,
    widgets: &mut HashMap<String, UiControl>,
    keys: &[Listed],
    busy: bool,
    height: f32,
    out: &mut Out,
) {
    card(ui, Some(height), |ui| {
        let (bar, _) = ui.allocate_exact_size(vec2(ui.available_width(), 36.0), Sense::hover());
        let painter = ui.painter();
        painter.text(
            pos2(bar.left() + 12.0, bar.center().y),
            Align2::LEFT_CENTER,
            format!("KEYS · {}", keys.len()),
            semibold(11.0),
            FAINT(),
        );
        let place = painter.text(
            pos2(bar.right() - 12.0, bar.center().y),
            Align2::RIGHT_CENTER,
            file_name(&s.scan_directory),
            FontId::proportional(12.0),
            HINT(),
        );
        painter.hline(bar.x_range(), bar.bottom() - 0.5, Stroke::new(1.0, CARD_LINE()));
        ui.interact(place, Id::new("ssh-folder-name"), Sense::hover())
            .on_hover_text(crate::views::display_path(&s.scan_directory));
        let can_tick = s.loaded && s.agent_running && !busy;
        let rows = (ui.available_height() - 48.0).max(52.0);
        let scroll = ScrollArea::vertical()
            .id_salt("ssh-keys-scroll")
            .max_height(rows)
            .min_scrolled_height(rows)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                if keys.is_empty() {
                    ui.add_space(16.0);
                    ui.vertical_centered(|ui| {
                        ui.label(RichText::new("No keys yet").size(12.5).color(HINT()));
                    });
                }
                let chosen = s.source();
                for k in keys {
                    let selected = chosen == Some(k.source);
                    let unlocked = s.loaded_keys.contains(&k.fingerprint);
                    let (row, tick) = key_row(ui, k, selected, unlocked, can_tick);
                    control(widgets, k.source.id(), &row, ui.clip_rect());
                    // The chosen key's tick keeps the names of the Unlock and Unload buttons it replaces.
                    let tick_id = match (selected, unlocked) {
                        (true, false) => "ssh-unlock".to_owned(),
                        (true, true) => "ssh-unload".to_owned(),
                        (false, _) => k.source.tick_id(),
                    };
                    // Disabled, like those buttons, while ssh-agent can't take it.
                    pages::region(widgets, &tick_id, tick.rect.intersect(ui.clip_rect()), can_tick);
                    if tick.clicked() {
                        out.tick = Some((k.source, unlocked));
                    } else if row.clicked() && !busy {
                        out.pick = Some(k.source);
                    }
                }
            });
        pages::region(widgets, "ssh-keys-scroll", scroll.inner_rect.intersect(ui.clip_rect()), true);
        let (foot, _) = ui.allocate_exact_size(vec2(ui.available_width(), 48.0), Sense::hover());
        ui.painter().hline(foot.x_range(), foot.top() + 0.5, Stroke::new(1.0, CARD_LINE()));
        let mut row = ui.new_child(
            UiBuilder::new()
                .max_rect(foot.shrink2(vec2(12.0, 10.0)))
                .layout(Layout::left_to_right(Align::Center)),
        );
        row.spacing_mut().item_spacing.x = 8.0;
        let half = (row.available_width() - 8.0) / 2.0;
        let new = icon_btn(&mut row, Icon::Plus, "New key", !busy, false, half);
        control(widgets, "ssh-new", &new, row.clip_rect());
        let file = row.add_enabled(!busy, Button::new("Add key file").min_size(vec2(half, 28.0)));
        // Add key file keeps the name of the "Use an existing private key" box it replaces.
        control(widgets, "ssh-existing-toggle", &file, row.clip_rect());
        if new.clicked() {
            out.form = Some(Form::New);
        } else if file.clicked() {
            out.form = Some(Form::File);
        }
    });
}

/// One key: its tick, icon, name and place. Returns the row and its tick.
fn key_row(ui: &mut Ui, k: &Listed, selected: bool, unlocked: bool, can_tick: bool) -> (Response, Response) {
    let (rect, row) = ui.allocate_exact_size(vec2(ui.available_width(), 52.0), Sense::click());
    let tick_rect = Rect::from_center_size(pos2(rect.left() + 20.0, rect.center().y), vec2(16.0, 16.0));
    let tick = ui.interact(
        tick_rect.expand(5.0),
        row.id.with("tick"),
        if can_tick { Sense::click() } else { Sense::hover() },
    );
    let painter = ui.painter();
    if selected {
        painter.rect_filled(rect, 0.0, SELECTED());
        painter.rect_filled(Rect::from_min_size(rect.min, vec2(3.0, rect.height())), 0.0, BLUE());
    } else if row.hovered() || tick.hovered() {
        painter.rect_filled(rect, 0.0, LIST_HOT());
    }
    painter.hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0, CARD_LINE()));
    let edge = if unlocked {
        BLUE()
    } else if can_tick && tick.hovered() {
        EDGE_HOT()
    } else {
        BORDER()
    };
    painter.rect(tick_rect, 4.0, if unlocked { BLUE() } else { FIELD() }, Stroke::new(1.0, edge), StrokeKind::Inside);
    if unlocked {
        icon(painter, tick_rect.shrink(2.0), Icon::Check, ACCENT_INK());
    }
    let tile = Rect::from_min_size(pos2(rect.left() + 38.0, rect.center().y - 14.0), vec2(28.0, 28.0));
    painter.rect(tile, 7.0, RAISED(), Stroke::new(1.0, BORDER()), StrokeKind::Inside);
    icon(painter, tile.shrink(7.0), Icon::Key, if selected { BLUE() } else { MUTED() });
    let left = tile.right() + 10.0;
    let right = rect.right() - 10.0;
    clipped_text(ui, Rect::from_min_max(pos2(left, rect.top() + 8.0), pos2(right, rect.center().y + 1.0)), &k.name, 13.0, TEXT());
    clipped_text(ui, Rect::from_min_max(pos2(left, rect.center().y + 1.0), pos2(right, rect.bottom() - 7.0)), &k.detail, 12.0, HINT());
    row.widget_info(|| WidgetInfo::selected(WidgetType::SelectableLabel, true, selected, &k.name));
    tick.widget_info(|| WidgetInfo::selected(WidgetType::Checkbox, can_tick, unlocked, "Unlocked in ssh-agent"));
    (row, tick)
}

/// A framed button with an icon before its label, at least `width` wide.
fn icon_btn(ui: &mut Ui, image: Icon, label: &str, enabled: bool, primary: bool, width: f32) -> Response {
    let ink = if primary { ACCENT_INK() } else { TEXT() };
    // A disabled Ui fades its painter already; a disabled button alone doesn't.
    let ink = if !enabled && ui.is_enabled() { ink.gamma_multiply(0.45) } else { ink };
    let font = if primary { semibold(13.0) } else { FontId::proportional(13.0) };
    let galley = ui.painter().layout_no_wrap(label.to_owned(), font, ink);
    let content = 13.0 + 6.0 + galley.size().x;
    let mut button = Button::new("").min_size(vec2(width.max(content + 22.0), 28.0));
    if primary {
        button = button.fill(BLUE());
    }
    let r = ui.add_enabled(enabled, button);
    let left = r.rect.center().x - content / 2.0;
    icon(ui.painter(), Rect::from_center_size(pos2(left + 6.5, r.rect.center().y), vec2(13.0, 13.0)), image, ink);
    ui.painter().galley(pos2(left + 19.0, r.rect.center().y - galley.size().y / 2.0), galley, ink);
    r.widget_info(|| WidgetInfo::labeled(WidgetType::Button, enabled, label));
    r
}

/// The right side: a passphrase question, a review, a key form or the chosen key.
#[allow(clippy::too_many_arguments)]
fn right_side(
    ui: &mut Ui,
    s: &mut State,
    widgets: &mut HashMap<String, UiControl>,
    keys: &[Listed],
    repository: &Option<String>,
    busy: bool,
    height: Option<f32>,
    out: &mut Out,
) {
    let chosen = keys.iter().find(|k| Some(k.source) == s.source());
    if let Some(prompt) = s.prompt {
        ui.spacing_mut().item_spacing.y = 12.0;
        if s.creating {
            ui.add_enabled_ui(false, |ui| form_card(ui, s, widgets, repository, out));
        }
        let name = chosen.map_or("the key", |k| k.name.as_str());
        prompt_card(ui, s, widgets, prompt, name);
    } else if s.preview.is_some() {
        review_card(ui, s, widgets, chosen, repository, out);
    } else if s.creating {
        ui.add_enabled_ui(!busy, |ui| form_card(ui, s, widgets, repository, out));
    } else if let Some(k) = chosen {
        ui.add_enabled_ui(!busy, |ui| key_card(ui, s, widgets, k, repository, height, out));
    }
}

/// The chosen key: where it is, its fingerprint, its public key with Copy, and the open
/// repository's use of it.
fn key_card(
    ui: &mut Ui,
    s: &mut State,
    widgets: &mut HashMap<String, UiControl>,
    k: &Listed,
    repository: &Option<String>,
    height: Option<f32>,
    out: &mut Out,
) {
    card(ui, height, |ui| {
        let kind = match k.encrypted {
            Some(true) => format!("{} · passphrase", k.kind),
            Some(false) => format!("{} · no passphrase", k.kind),
            None => k.kind.clone(),
        };
        card_header(ui, Icon::Key, &k.name, &kind, |ui| {
            if s.loaded && s.agent_running {
                if s.loaded_keys.contains(&k.fingerprint) {
                    pages::pill(ui, "In agent", GREEN());
                } else {
                    pages::pill(ui, "Not in agent", HINT());
                }
            }
        });
        Frame::new().inner_margin(Margin { left: 16, right: 16, top: 14, bottom: 14 }).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 10.0;
            let path = crate::views::display_path(&k.path);
            if info_row(ui, widgets, "Location", &path, ("ssh-show-folder", Icon::Folder, "Show in File Explorer")) {
                out.reveal = k.path.parent().map(Path::to_path_buf);
            }
            if info_row(ui, widgets, "Fingerprint", &k.fingerprint, ("ssh-copy-fingerprint", Icon::Copy, "Copy fingerprint")) {
                out.copy = Some((k.fingerprint.clone(), "Fingerprint copied."));
            }
            ui.label(RichText::new("Public key").size(12.0).color(HINT()));
            // Read-only, so it can be selected; an RSA key scrolls instead of growing.
            ScrollArea::vertical().id_salt("ssh-public-key").max_height(62.0).show(ui, |ui| {
                let mut public = k.public_key.as_str();
                ui.add(
                    TextEdit::multiline(&mut public)
                        .font(FontId::monospace(12.0))
                        .desired_rows(3)
                        .desired_width(f32::INFINITY),
                );
            });
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 12.0;
                let r = icon_btn(ui, Icon::Copy, "Copy public key", true, true, 0.0);
                control(widgets, "ssh-copy", &r, ui.clip_rect());
                if r.clicked() {
                    out.copy = Some((k.public_key.clone(), "Public key copied. The private key was not copied."));
                }
                ui.add(Label::new(RichText::new("Paste it into your Git host's SSH key settings.").size(12.0).color(HINT())).truncate());
            });
            ui.separator();
            ui.label(RichText::new("Use this key through").font(semibold(13.0)));
            ui.horizontal_wrapped(|ui| {
                for (id,label,mode) in [("ssh-mode-agent",if cfg!(windows) {"Windows agent"} else {"ssh-agent"},SshAuthentication::Agent),("ssh-mode-direct","Direct key",SshAuthentication::Direct)] {
                    let r=ui.selectable_value(&mut s.authentication,mode,label);
                    control(widgets,id,&r,ui.clip_rect());
                }
            });
            if s.authentication==SshAuthentication::Direct {
                ui.label(RichText::new("Uses this key directly. Unknown or changed host keys are rejected.").size(12.0).color(HINT()));
                if let Some(p)=s.profile() {
                    let saved=p.passphrase_saved;
                    let saved_text = if cfg!(windows) {"Passphrase saved with Windows user encryption."} else {"Passphrase saved, encrypted with a key in your desktop keyring."};
                    ui.label(RichText::new(if saved {saved_text} else if p.encrypted {"Save this key's passphrase before using it directly."} else {"This key has no passphrase."}).size(12.0).color(if saved {GREEN()}else{HINT()}));
                    ui.horizontal_wrapped(|ui| {
                        if btn(ui,widgets,"ssh-remember","Save passphrase",p.encrypted&&!saved){out.action=Some("remember");}
                        if btn(ui,widgets,"ssh-forget","Forget passphrase",saved){out.action=Some("forget");}
                    });
                } else {
                    ui.label(RichText::new("First use the repository action below to set up this key's host profile.").size(12.0).color(HINT()));
                }
            } else {
                ui.label(RichText::new(if cfg!(windows) {"The key's tick controls Windows ssh-agent. Rocket does not save its passphrase in this mode."} else {"The key's tick controls ssh-agent. Rocket does not save its passphrase in this mode."}).size(12.0).color(HINT()));
            }
            // The repository box sits at the card's foot when the card fills the page. Its
            // buttons go beside the repository when they fit, otherwise under it.
            let beside = buttons_beside(ui, k);
            if height.is_some() {
                ui.add_space((ui.available_height() - if beside.is_some() { 82.0 } else { 120.0 }).max(0.0));
            }
            repository_box(ui, s, widgets, k, repository, beside, out);
        });
    });
}

/// A labelled value on one line with an icon button after it. True when it was clicked.
fn info_row(
    ui: &mut Ui,
    widgets: &mut HashMap<String, UiControl>,
    label: &str,
    value: &str,
    (key, image, hint): (&str, Icon, &str),
) -> bool {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 12.0;
        let (rect, _) = ui.allocate_exact_size(vec2(76.0, 24.0), Sense::hover());
        ui.painter().text(rect.left_center(), Align2::LEFT_CENTER, label, FontId::proportional(12.0), HINT());
        let width = (ui.available_width() - 38.0).max(40.0);
        ui.allocate_ui_with_layout(vec2(width, 24.0), Layout::left_to_right(Align::Center), |ui| {
            ui.set_width(width);
            ui.add(Label::new(RichText::new(value).monospace().size(12.5)).truncate())
                .on_hover_text(value);
        });
        let r = crate::chrome::icon_button(ui, image, true, false, 26.0).on_hover_text(hint);
        control(widgets, key, &r, ui.clip_rect());
        r.clicked()
    })
    .inner
}

/// The width of the repository box's buttons when they fit beside the repository and its
/// Restore link, or `None` when they go under it.
fn buttons_beside(ui: &Ui, k: &Listed) -> Option<f32> {
    let width = |text: &str, size: f32| {
        ui.painter().layout_no_wrap(text.to_owned(), FontId::proportional(size), TEXT()).size().x
    };
    // A button pads its label by 10 points a side; buttons are 8 apart.
    let mut buttons = width("Use only this key", 13.0) + 20.0;
    if let Source::Profile(_) = k.source {
        buttons += 8.0 + width("Check settings", 13.0) + 20.0;
    }
    // The box's margins and edge take 26 points, the gap before the buttons 16.
    let room = ui.available_width() - 26.0 - 16.0;
    (room >= width("Restore previous setting", 12.0) + buttons).then_some(buttons)
}

/// The open repository and this key: Use only this key, Check settings, Restore.
#[allow(clippy::too_many_arguments)]
fn repository_box(
    ui: &mut Ui,
    s: &State,
    widgets: &mut HashMap<String, UiControl>,
    k: &Listed,
    repository: &Option<String>,
    beside: Option<f32>,
    out: &mut Out,
) {
    let about = |ui: &mut Ui, widgets: &mut HashMap<String, UiControl>, out: &mut Out| {
        ui.spacing_mut().item_spacing.y = 2.0;
        ui.label(RichText::new("THIS REPOSITORY").font(semibold(11.0)).color(FAINT()));
        match repository {
            Some(name) => {
                ui.add(Label::new(RichText::new(name).size(13.0)).truncate());
                let r = restore_link(ui, "Restore previous setting", true);
                control(widgets, "ssh-restore", &r, ui.clip_rect());
                if r.clicked() {
                    out.action = Some("restore");
                }
            }
            None => {
                ui.add(Label::new(RichText::new("Open a repository to use only this key with it.").size(12.5).color(HINT())).wrap());
            }
        }
    };
    let buttons = |ui: &mut Ui, widgets: &mut HashMap<String, UiControl>, out: &mut Out| {
        ui.spacing_mut().item_spacing.x = 8.0;
        if btn(ui, widgets, "ssh-review", "Use only this key", s.loaded && repository.is_some()) {
            match k.source {
                Source::Profile(_) => out.action = Some("review"),
                Source::Found(i) => out.form = Some(Form::Use(i)),
            }
        }
        // `ssh -G` reads the profile's settings; it doesn't connect.
        if let Source::Profile(_) = k.source {
            let r = pages::ghost(ui, true, "Check settings");
            control(widgets, "ssh-check", &r, ui.clip_rect());
            if r.clicked() {
                out.action = Some("check");
            }
        }
    };
    Frame::new()
        .fill(CARD())
        .stroke(Stroke::new(1.0, CARD_LINE()))
        .corner_radius(8)
        .inner_margin(Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            match beside {
                Some(width) => {
                    ui.horizontal(|ui| {
                        let text = ui.available_width() - width - 16.0;
                        ui.allocate_ui_with_layout(vec2(text, 57.0), Layout::top_down(Align::Min), |ui| about(ui, widgets, out));
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| buttons(ui, widgets, out));
                    });
                }
                None => {
                    ui.spacing_mut().item_spacing.y = 10.0;
                    ui.vertical(|ui| about(ui, widgets, out));
                    ui.horizontal(|ui| buttons(ui, widgets, out));
                }
            }
        });
}

/// New key, Add key file, or a found key's settings before the repository uses it.
fn form_card(
    ui: &mut Ui,
    s: &mut State,
    widgets: &mut HashMap<String, UiControl>,
    repository: &Option<String>,
    out: &mut Out,
) {
    let found = s.discovered_choice.clone();
    let root = crate::views::display_path(&s.root);
    let (image, title, detail, submit) = match (&found, s.use_existing) {
        (Some(key), _) => (
            Icon::Key,
            format!("Use {} for {}", file_name(&key.key), repository.as_deref().unwrap_or("the repository")),
            key.fingerprint.clone(),
            "Continue",
        ),
        (None, true) => (Icon::Plus, "Add key file".to_owned(), "The key file stays where it is.".to_owned(), "Add key"),
        (None, false) => (Icon::Plus, "New key".to_owned(), format!("Saved in {root}"), "Create key"),
    };
    card(ui, None, |ui| {
        card_header(ui, image, &title, &detail, |_| {});
        Frame::new().inner_margin(Margin { left: 16, right: 16, top: 14, bottom: 14 }).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 6.0;
            if s.use_existing && found.is_none() {
                field(ui, widgets, "ssh-existing", "Key file (its .pub file must be beside it)", &mut s.existing, true, false);
                ui.add_space(4.0);
            }
            field(ui, widgets, "ssh-alias", "Name (rocket-…)", &mut s.alias, true, false);
            if !s.use_existing {
                ui.add_space(4.0);
                ui.label("Type");
                let r = pages::segments(ui, &[("Ed25519 · recommended", !s.rsa), ("RSA 4096", s.rsa)]);
                control(widgets, "ssh-ed25519", &r[0], ui.clip_rect());
                control(widgets, "ssh-rsa", &r[1], ui.clip_rect());
                if r[0].clicked() {
                    s.rsa = false;
                } else if r[1].clicked() {
                    s.rsa = true;
                }
            }
            ui.add_space(4.0);
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                let total = ui.available_width() - 20.0;
                let (user, port) = (total * 0.28, total * 0.17);
                for (key, label, value, width) in [
                    ("ssh-host", "Host", &mut s.host, total - user - port),
                    ("ssh-user", "User", &mut s.user, user),
                    ("ssh-port", "Port", &mut s.port, port),
                ] {
                    ui.allocate_ui_with_layout(vec2(width, 52.0), Layout::top_down(Align::Min), |ui| {
                        ui.set_width(width);
                        ui.spacing_mut().item_spacing.y = 6.0;
                        field(ui, widgets, key, label, value, true, false);
                    });
                }
            });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                if primary_btn(ui, widgets, "ssh-create", submit, true) {
                    out.action = Some("create");
                }
                if btn(ui, widgets, "ssh-form-cancel", "Cancel", true) {
                    s.creating = false;
                    s.discovered_choice = None;
                    s.review_after_create = false;
                }
            });
        });
    });
}

/// What OpenSSH asks while it makes or unlocks a key. The passphrase goes straight to it.
fn prompt_card(ui: &mut Ui, s: &mut State, widgets: &mut HashMap<String, UiControl>, prompt: Prompt, name: &str) {
    let title = match prompt {
        Prompt::New | Prompt::Confirm | Prompt::NewRetry => "ssh-keygen asks for a passphrase",
        Prompt::Unlock | Prompt::UnlockRetry => "ssh-add asks for the passphrase",
        Prompt::Remember => "Verify and save this key's passphrase",
    };
    let label = match prompt {
        Prompt::New => "Enter a new passphrase, at least 10 characters".to_owned(),
        Prompt::Confirm => "Confirm the passphrase".to_owned(),
        Prompt::NewRetry => "The passphrases didn't match. Enter a new passphrase.".to_owned(),
        Prompt::Unlock => format!("Passphrase for {name}"),
        Prompt::UnlockRetry => "Passphrase rejected. Try again.".to_owned(),
        Prompt::Remember => format!("Passphrase for {name} — saved only after OpenSSH verifies it"),
    };
    pages::titled_card(
        ui,
        |ui| {
            ui.label(RichText::new(title).font(semibold(13.0)));
        },
        |ui| {
            field(ui, widgets, "ssh-passphrase", &label, &mut s.passphrase, !s.stopping, true);
            ui.add_space(4.0);
            if primary_btn(ui, widgets, "ssh-send", "Send securely", !s.stopping) {
                send_passphrase(s, prompt);
            }
        },
    );
}

/// Hand the typed passphrase to OpenSSH when it is acceptable for `prompt`.
fn send_passphrase(s: &mut State, prompt: Prompt) {
    let valid = s.passphrase.len() <= 1024
        && !s.passphrase.chars().any(char::is_control)
        && (matches!(prompt, Prompt::Unlock | Prompt::UnlockRetry | Prompt::Remember) || s.passphrase.chars().count() >= 10);
    if !valid {
        s.error = Some("Use at least 10 characters for a new key, at most 1024 bytes, without control characters.".into());
    } else if s.input.as_ref().is_some_and(|c| c.try_send(std::mem::take(&mut s.passphrase)).is_ok()) {
        s.prompt = None;
        s.error = None;
    }
}

/// Using only the chosen key for the open repository, or restoring its previous setting.
fn review_card(
    ui: &mut Ui,
    s: &mut State,
    widgets: &mut HashMap<String, UiControl>,
    chosen: Option<&Listed>,
    repository: &Option<String>,
    out: &mut Out,
) {
    let Some(review) = s.preview.clone() else { return };
    let repository = repository.as_deref().unwrap_or("the repository");
    let name = match (&review.profile, chosen) {
        (Some(p), Some(k)) if same_file(&p.key, &k.path) => k.name.clone(),
        (Some(p), _) => p.alias.clone(),
        (None, _) => String::new(),
    };
    let title = if review.restoring {
        format!("Restore the previous SSH setting of {repository}")
    } else {
        format!("Use only {name} for {repository}")
    };
    pages::titled_card(
        ui,
        |ui| {
            ui.add(Label::new(RichText::new(title).font(semibold(13.0))).truncate());
        },
        |ui| {
            pages::key_value(ui, "Repository", 96.0, |ui| {
                ui.label(repository);
            });
            if let Some(p) = &review.profile {
                pages::key_value(ui,"Authentication",96.0,|ui|{ui.label(if review.authentication==SshAuthentication::Direct {"Direct key · host verification required"}else if cfg!(windows) {"Windows agent / OpenSSH"} else {"ssh-agent / OpenSSH"});});
                pages::key_value(ui, "Key", 96.0, |ui| {
                    ui.label(&name);
                });
                pages::key_value(ui, "Connects as", 96.0, |ui| {
                    ui.label(format!("{}@{}:{}", p.user, p.host, p.port));
                });
                pages::key_value(ui, "Fingerprint", 96.0, |ui| {
                    ui.label(RichText::new(&p.fingerprint).monospace().size(12.5));
                });
            }
            if !review.restoring {
                pages::callout(
                    ui,
                    AMBER(),
                    "The repository will use this key's own SSH config, so proxies and other settings in yours won't apply to it.",
                );
            }
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                let apply = if review.restoring { "Restore setting" } else { "Use only this key" };
                if primary_btn(ui, widgets, "ssh-apply", apply, true) {
                    out.action = Some("apply");
                }
                if btn(ui, widgets, "ssh-back", "Back", true) {
                    s.preview = None;
                }
            });
        },
    );
}

/// No keys in the SSH folder and none made by Rocket. The open repository's previous
/// setting can still be restored, since that needs no key.
fn empty_state(
    ui: &mut Ui,
    s: &State,
    widgets: &mut HashMap<String, UiControl>,
    repository: &Option<String>,
    busy: bool,
    out: &mut Out,
) {
    let height = (ui.available_height() - 2.0).max(260.0);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    ui.painter().rect_filled(rect, 8.0, PANEL());
    ui.painter()
        .extend(Shape::dashed_line(&rounded_outline(rect.shrink(0.5), 8.0), Stroke::new(1.0, BORDER()), 5.0, 4.0));
    let mut inner = ui.new_child(
        UiBuilder::new()
            .max_rect(rect.shrink(24.0))
            .layout(Layout::top_down(Align::Center)),
    );
    let ui = &mut inner;
    ui.spacing_mut().item_spacing.y = 12.0;
    // The tile, two lines and the buttons, centered as one group.
    ui.add_space(((rect.height() - 48.0 - 168.0) / 2.0).max(0.0));
    let (tile, _) = ui.allocate_exact_size(vec2(48.0, 48.0), Sense::hover());
    ui.painter().rect(tile, 12.0, RAISED(), Stroke::new(1.0, EDGE_HOT()), StrokeKind::Inside);
    icon(ui.painter(), tile.shrink(13.0), Icon::Key, BLUE());
    let folder = crate::views::display_path(&s.scan_directory);
    ui.add(Label::new(RichText::new(format!("No SSH keys in {folder}")).font(semibold(15.0))).wrap());
    ui.label(RichText::new("Create one here, or add a key file from another folder.").color(MUTED()));
    ui.add_space(2.0);
    ui.allocate_ui_with_layout(vec2(248.0, 28.0), Layout::left_to_right(Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let new = icon_btn(ui, Icon::Plus, "New key", !busy, true, 120.0);
        control(widgets, "ssh-new", &new, ui.clip_rect());
        let file = ui.add_enabled(!busy, Button::new("Add key file").min_size(vec2(120.0, 28.0)));
        control(widgets, "ssh-existing-toggle", &file, ui.clip_rect());
        if new.clicked() {
            out.form = Some(Form::New);
        } else if file.clicked() {
            out.form = Some(Form::File);
        }
    });
    if let Some(repository) = repository {
        let r = restore_link(ui, &format!("Restore the previous SSH setting of {repository}"), !busy);
        control(widgets, "ssh-restore", &r, ui.clip_rect());
        if r.clicked() {
            out.action = Some("restore");
        }
    }
}

/// A Restore link: blue text, flush with the text above it, on a frameless button tall
/// enough to hit easily.
fn restore_link(ui: &mut Ui, text: &str, enabled: bool) -> Response {
    ui.scope(|ui| {
        ui.spacing_mut().button_padding = vec2(0.0, 4.0);
        let link = Button::new(RichText::new(text).size(12.0).color(BLUE())).frame(false).truncate();
        ui.add_enabled(enabled, link.min_size(vec2(0.0, 22.0)))
    })
    .inner
    .on_hover_cursor(CursorIcon::PointingHand)
}

/// Points around `rect` with rounded corners, for a dashed edge.
fn rounded_outline(rect: Rect, radius: f32) -> Vec<Pos2> {
    let corners = [
        (pos2(rect.right() - radius, rect.top() + radius), -90.0_f32),
        (pos2(rect.right() - radius, rect.bottom() - radius), 0.0),
        (pos2(rect.left() + radius, rect.bottom() - radius), 90.0),
        (pos2(rect.left() + radius, rect.top() + radius), 180.0),
    ];
    let mut points = Vec::with_capacity(4 * 7 + 1);
    for (center, start) in corners {
        for step in 0..7 {
            let angle = (start + 15.0 * step as f32).to_radians();
            points.push(center + radius * vec2(angle.cos(), angle.sin()));
        }
    }
    points.push(points[0]);
    points
}

/// The user (when the URL names one), host and port of the first SSH remote, origin first.
fn remote_endpoint(remotes: &[Remote]) -> Option<(Option<String>, String, u16)> {
    let origin = remotes.iter().filter(|r| r.name == b"origin");
    origin
        .chain(remotes.iter().filter(|r| r.name != b"origin"))
        .flat_map(|r| &r.urls)
        .find_map(|url| std::str::from_utf8(url).ok().and_then(ssh_endpoint))
}

/// The user (when it names one), host and port an SSH remote URL connects to:
/// `[user@]host:path` or `ssh://[user@]host[:port]/path`. Other URLs and local paths
/// give `None`.
fn ssh_endpoint(url: &str) -> Option<(Option<String>, String, u16)> {
    let url = url.trim();
    let split = |authority: &str| match authority.rsplit_once('@') {
        Some((user, host)) => (Some(user.to_owned()), host.to_owned()),
        None => (None, authority.to_owned()),
    };
    let valid = |user: &Option<String>, host: &str| {
        !host.is_empty()
            && !host.contains(['[', ']', ' ', '\\'])
            && user.as_ref().is_none_or(|u| !u.is_empty() && !u.contains(' '))
    };
    let ssh = ["ssh://", "git+ssh://", "ssh+git://"].iter().find_map(|p| url.strip_prefix(p));
    if let Some(rest) = ssh {
        let (user, authority) = split(rest.split('/').next()?);
        let (host, port) = match authority.rsplit_once(':') {
            Some((host, port)) => (host.to_owned(), port.parse().ok()?),
            None => (authority, 22),
        };
        return valid(&user, &host).then_some((user, host, port));
    }
    if url.contains("://") {
        return None;
    }
    // scp-like syntax has no slash before its colon and no port.
    let (authority, _) = url.split_once(':')?;
    if authority.contains(['/', '\\']) {
        return None;
    }
    let (user, host) = split(authority);
    // A drive letter, as in C:\repo, is a local path.
    if user.is_none() && host.len() == 1 {
        return None;
    }
    valid(&user, &host).then_some((user, host, 22))
}

/// A profile name for a key file: "rocket-" and the name's letters and digits in lower
/// case, anything else as single dashes.
fn alias_for(name: &str) -> String {
    let lower = name.to_ascii_lowercase();
    let name = lower.strip_prefix("rocket-").unwrap_or(&lower);
    let mut alias = String::from("rocket-");
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            alias.push(c);
        } else if !alias.ends_with('-') {
            alias.push('-');
        }
    }
    // Profile names are at most 64 characters, all ASCII here.
    alias.truncate(64);
    let alias = alias.trim_end_matches('-');
    if alias == "rocket" { "rocket-key".to_owned() } else { alias.to_owned() }
}

/// A path's last part for display, or the whole path when it has none.
fn file_name(path: &Path) -> String {
    path.file_name()
        .map_or_else(|| crate::views::display_path(path), |n| n.to_string_lossy().into_owned())
}

/// A path as the system compares it: on Windows without `\\?\`, with backslashes,
/// ignoring case; elsewhere exactly as written, since case matters.
fn comparable(path: &Path) -> String {
    if cfg!(windows) {
        crate::views::display_path(path).replace('/', "\\").to_lowercase()
    } else {
        path.to_string_lossy().into_owned()
    }
}

fn same_file(a: &Path, b: &Path) -> bool {
    comparable(a) == comparable(b)
}

fn under(path: &Path, root: &Path) -> bool {
    let separator = std::path::MAIN_SEPARATOR;
    let (path, root) = (comparable(path), comparable(root));
    path.strip_prefix(root.trim_end_matches(separator)).is_some_and(|rest| rest.starts_with(separator))
}

/// "Ed25519", "RSA" or "ECDSA" for an OpenSSH key type such as `ssh-ed25519`.
fn kind_label(kind: &str) -> String {
    match kind {
        "ssh-ed25519" => "Ed25519".into(),
        "ssh-rsa" => "RSA".into(),
        k if k.starts_with("ecdsa-") => "ECDSA".into(),
        "" => "Key".into(),
        k => k.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ssh_endpoint_reads_ssh_remotes_only() {
        let at = |user: Option<&str>, host: &str, port| Some((user.map(str::to_owned), host.to_owned(), port));
        assert_eq!(ssh_endpoint("git@github.com:owner/repo.git"), at(Some("git"), "github.com", 22));
        assert_eq!(ssh_endpoint("github.com:owner/repo"), at(None, "github.com", 22));
        assert_eq!(
            ssh_endpoint("ssh://deploy@git.example.com:2222/owner/repo"),
            at(Some("deploy"), "git.example.com", 2222)
        );
        assert_eq!(ssh_endpoint("ssh://git.example.com/repo"), at(None, "git.example.com", 22));
        assert_eq!(ssh_endpoint("git+ssh://git@host.test/repo"), at(Some("git"), "host.test", 22));
        for other in [
            "https://github.com/owner/repo.git",
            "file:///srv/repo",
            r"C:\repos\one",
            "C:/repos/one",
            "../sibling",
            "/srv/git/repo",
            r"\\server\share\repo",
            "ssh://[::1]/repo",
            "ssh://host:port/repo",
            "ssh:///repo",
            "",
        ] {
            assert_eq!(ssh_endpoint(other), None, "{other}");
        }
    }

    #[test]
    fn alias_for_makes_valid_profile_names() {
        assert_eq!(alias_for("id_ed25519_github"), "rocket-id-ed25519-github");
        assert_eq!(alias_for("Work Key.private"), "rocket-work-key-private");
        assert_eq!(alias_for("工作.key"), "rocket-key");
        assert_eq!(alias_for("Rocket-Work"), "rocket-work");
        assert_eq!(alias_for("--"), "rocket-key");
        let long = alias_for(&format!("{}-b", "a".repeat(56)));
        assert_eq!(long, format!("rocket-{}", "a".repeat(56)));
        assert_eq!(alias_for(&"x".repeat(100)).len(), 64);
    }

    #[cfg(not(windows))]
    #[test]
    fn linux_paths_compare_exactly() {
        assert!(same_file(Path::new("/home/a/.ssh/id"), Path::new("/home/a/.ssh/id")));
        assert!(!same_file(Path::new("/home/a/.ssh/Id"), Path::new("/home/a/.ssh/id")), "case matters");
        assert!(under(Path::new("/home/a/.ssh/rocket-keys/w/id"), Path::new("/home/a/.ssh/rocket-keys/")));
        assert!(!under(Path::new("/home/a/.ssh/rocket-keys-old/id"), Path::new("/home/a/.ssh/rocket-keys")));
    }

    #[cfg(windows)]
    #[test]
    fn paths_compare_as_windows_does() {
        assert!(same_file(Path::new(r"\\?\C:\Users\A\.ssh\id"), Path::new(r"c:\users\a\.SSH\id")));
        assert!(same_file(Path::new("C:/Users/A/.ssh/id"), Path::new(r"C:\Users\A\.ssh\id")));
        assert!(!same_file(Path::new(r"C:\a\id"), Path::new(r"C:\a\id2")));
        assert!(under(Path::new(r"C:\a\.ssh\rocket-keys\w\id"), Path::new(r"\\?\C:\A\.ssh\rocket-keys")));
        assert!(!under(Path::new(r"C:\a\.ssh\rocket-keys-old\id"), Path::new(r"C:\a\.ssh\rocket-keys")));
        assert_eq!(kind_label("ecdsa-sha2-nistp256"), "ECDSA");
        assert_eq!(kind_label("ssh-ed25519"), "Ed25519");
    }

    /// The page from an empty SSH folder to a found key and its profile: the controls the
    /// runners use, the copied public key, one row per key, and both layouts.
    #[cfg(windows)]
    #[test]
    fn page_lists_each_found_key_once_and_copies_its_public_key() {
        use eframe::egui::{Context, Event, Modifiers, OutputCommand, PointerButton, Pos2, RawInput, Rect, Vec2, vec2};
        use std::{
            os::windows::process::CommandExt,
            process::Command,
            time::{Duration, Instant},
        };
        crate::test_common::isolated(
            "ROCKET_SSH_PAGE_CASE",
            "ssh_keys::page::tests::page_lists_each_found_key_once_and_copies_its_public_key",
            || {
                let f = crate::test_common::Fixture::new();
                let ctx = Context::default();
                let mut app = Rocket::with_storage(&ctx, vec![], f.root.join("logs"), f.root.join("state"));
                // A disposable SSH folder: nothing of the user's own is read.
                let base = std::fs::canonicalize(&f.root).unwrap();
                let folder = PathBuf::from(base.to_string_lossy().trim_start_matches(r"\\?\")).join("ssh");
                std::fs::create_dir(&folder).unwrap();
                app.ssh.scan_directory = folder.clone();
                app.ssh.root = folder.join("rocket-keys");
                app.ssh.loaded = true;
                let settle = |app: &mut Rocket| {
                    let deadline = Instant::now() + Duration::from_secs(30);
                    while app.backend.is_none() || app.busy() || app.ssh.running() {
                        app.poll(&ctx);
                        assert!(Instant::now() < deadline, "Rocket did not settle");
                        std::thread::sleep(Duration::from_millis(5));
                    }
                };
                settle(&mut app);
                app.show_preferences(crate::preferences::Page::SshKeys);
                let frame = |app: &mut Rocket, size: Vec2, events: Vec<Event>| {
                    app.poll(&ctx);
                    app.frame_widgets.clear();
                    let raw = RawInput {
                        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
                        events,
                        ..Default::default()
                    };
                    let mut out = ctx.run_ui(raw, |ui| app.dialog_ui(ui.ctx()));
                    out.textures_delta.clear();
                    std::mem::take(&mut out.platform_output.commands)
                };
                let wide = vec2(1280.0, 860.0);
                let shown = |app: &mut Rocket, keys: &[&str]| {
                    for _ in 0..3 {
                        frame(app, wide, vec![]);
                    }
                    for key in keys {
                        assert!(app.frame_widgets.contains_key(*key), "{key} is not shown");
                    }
                };
                // Clicks `key` and returns what that frame copied.
                let click = |app: &mut Rocket, key: &str| {
                    frame(app, wide, vec![]);
                    let control = app.frame_widgets.get(key).copied().unwrap_or_else(|| panic!("{key} is not shown"));
                    assert!(control.enabled, "{key} is disabled");
                    let pos = control.rect.center();
                    let press = |pressed| Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
                    frame(app, wide, vec![Event::PointerMoved(pos), press(true), press(false)])
                        .into_iter()
                        .filter_map(|command| match command {
                            OutputCommand::CopyText(text) => Some(text),
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                };

                // No keys: the page offers New key and Add key file.
                shown(&mut app, &["ssh-new", "ssh-existing-toggle", "ssh-scan"]);
                assert!(!app.frame_widgets.contains_key("ssh-keys-scroll"));
                assert!(!app.frame_widgets.contains_key("ssh-restore"), "no repository is open");
                click(&mut app, "ssh-new");
                shown(&mut app, &["ssh-keys-scroll", "ssh-alias", "ssh-host", "ssh-user", "ssh-port", "ssh-ed25519", "ssh-rsa", "ssh-create", "ssh-form-cancel"]);
                assert!(!app.frame_widgets.contains_key("ssh-existing"));
                click(&mut app, "ssh-existing-toggle");
                shown(&mut app, &["ssh-existing", "ssh-alias", "ssh-create"]);
                assert!(!app.frame_widgets.contains_key("ssh-rsa"), "a key file keeps its own type");
                click(&mut app, "ssh-form-cancel");
                shown(&mut app, &["ssh-new"]);
                assert!(!app.frame_widgets.contains_key("ssh-alias"));

                // A key in the folder: listed after Rescan and chosen; Copy copies its public key.
                let key = folder.join("id_found");
                let keygen = PathBuf::from(std::env::var_os("WINDIR").unwrap()).join("System32/OpenSSH/ssh-keygen.exe");
                let out = Command::new(keygen)
                    .args(["-q", "-t", "ed25519", "-N", "disposable-page-passphrase", "-C", "rocket-page-fixture", "-f"])
                    .arg(&key)
                    .creation_flags(0x0800_0000)
                    .output()
                    .unwrap();
                assert!(out.status.success(), "generate disposable key");
                // Rescan while New key is open: the found key is chosen and the form stays.
                click(&mut app, "ssh-new");
                click(&mut app, "ssh-scan");
                settle(&mut app);
                shown(&mut app, &["ssh-found-0", "ssh-alias"]);
                assert_eq!((app.ssh.selected, app.ssh.found), (None, Some(0)));
                click(&mut app, "ssh-form-cancel");
                shown(&mut app, &["ssh-found-0", "ssh-unlock", "ssh-copy", "ssh-copy-fingerprint", "ssh-show-folder", "ssh-review"]);
                assert_eq!((app.ssh.selected, app.ssh.found), (None, Some(0)));
                assert!(!app.frame_widgets["ssh-review"].enabled, "Use only this key needs a repository");
                assert!(!app.frame_widgets.contains_key("ssh-restore"), "no repository is open");
                assert!(!app.frame_widgets.contains_key("ssh-check"), "a found key has no settings to check");
                let row = app.frame_widgets["ssh-found-0"].rect;
                assert!(app.frame_widgets["ssh-show-folder"].rect.left() > row.right(), "the key is beside the list");
                // Its one button fits beside the repository, at the card's right edge as in the mock.
                let review = app.frame_widgets["ssh-review"].rect;
                let edge = app.frame_widgets["ssh-copy-fingerprint"].rect.right();
                assert!((review.right() - edge).abs() < 24.0, "Use only this key at {review:?}, card edge {edge}");
                let public = std::fs::read_to_string(folder.join("id_found.pub")).unwrap();
                assert_eq!(click(&mut app, "ssh-copy"), vec![public.trim().to_owned()]);
                assert!(app.ssh.notice.as_deref().is_some_and(|n| n.starts_with("Public key copied")));

                // Set up with Add key file, the key is listed once, as the profile.
                app.ssh.use_existing = true;
                app.ssh.existing = key.display().to_string();
                app.ssh.alias = "rocket-page".into();
                app.ssh.creating = true;
                click(&mut app, "ssh-create");
                settle(&mut app);
                assert_eq!(app.ssh.error, None);
                shown(&mut app, &["ssh-profile-0", "ssh-check", "ssh-copy"]);
                assert!(!app.frame_widgets.contains_key("ssh-found-0"));
                assert_eq!((app.ssh.selected, app.ssh.found), (Some(0), None));

                // A narrow window puts the list above the key.
                let narrow = vec2(780.0, 620.0);
                for _ in 0..3 {
                    frame(&mut app, narrow, vec![]);
                }
                let row = app.frame_widgets["ssh-profile-0"].rect;
                assert!(Rect::from_min_size(Pos2::ZERO, narrow).contains_rect(row));
                assert!(app.frame_widgets["ssh-show-folder"].rect.top() > row.bottom(), "the key is under the list");
            },
        );
    }

    /// With a repository open, Restore previous setting is a full-size target on the empty
    /// page and in a key's card, easy to hit (automation also needs it taller than 18 points).
    #[cfg(windows)]
    #[test]
    fn restore_previous_setting_is_a_full_size_target() {
        use eframe::egui::{Context, Pos2, RawInput, Rect, vec2};
        use std::{
            os::windows::process::CommandExt,
            process::Command,
            time::{Duration, Instant},
        };
        crate::test_common::isolated(
            "ROCKET_SSH_RESTORE_CASE",
            "ssh_keys::page::tests::restore_previous_setting_is_a_full_size_target",
            || {
                let f = crate::test_common::Fixture::new();
                f.write("file", "first");
                f.commit("first");
                let ctx = Context::default();
                let mut app = crate::batch::flow_tests::open(&ctx, &f);
                // A disposable SSH folder: nothing of the user's own is read.
                let base = std::fs::canonicalize(&f.root).unwrap();
                let folder = PathBuf::from(base.to_string_lossy().trim_start_matches(r"\\?\")).join("ssh");
                std::fs::create_dir(&folder).unwrap();
                app.ssh.scan_directory = folder.clone();
                app.ssh.root = folder.join("rocket-keys");
                app.ssh.loaded = true;
                app.show_preferences(crate::preferences::Page::SshKeys);
                let frames = |app: &mut Rocket| {
                    for _ in 0..3 {
                        app.poll(&ctx);
                        app.frame_widgets.clear();
                        let raw = RawInput {
                            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 860.0))),
                            ..Default::default()
                        };
                        let mut out = ctx.run_ui(raw, |ui| app.dialog_ui(ui.ctx()));
                        out.textures_delta.clear();
                    }
                };
                let restore = |app: &Rocket, place: &str| {
                    let c = app.frame_widgets.get("ssh-restore").copied().unwrap_or_else(|| panic!("no Restore on {place}"));
                    assert!(c.enabled && c.rect.height() >= 20.0, "Restore on {place} at {:?}", c.rect);
                };
                frames(&mut app);
                assert!(app.frame_widgets.contains_key("ssh-new"), "the empty page is shown");
                restore(&app, "the empty page");
                let keygen = PathBuf::from(std::env::var_os("WINDIR").unwrap()).join("System32/OpenSSH/ssh-keygen.exe");
                let out = Command::new(keygen)
                    .args(["-q", "-t", "ed25519", "-N", "disposable-restore-passphrase", "-C", "rocket-restore-fixture", "-f"])
                    .arg(folder.join("id_restore"))
                    .creation_flags(0x0800_0000)
                    .output()
                    .unwrap();
                assert!(out.status.success(), "generate disposable key");
                app.ssh_action("scan");
                let deadline = Instant::now() + Duration::from_secs(30);
                while app.ssh.running() {
                    app.poll(&ctx);
                    assert!(Instant::now() < deadline, "the scan did not finish");
                    std::thread::sleep(Duration::from_millis(5));
                }
                frames(&mut app);
                assert!(app.frame_widgets.contains_key("ssh-found-0"), "the found key is listed");
                restore(&app, "the key card");
            },
        );
    }
}
