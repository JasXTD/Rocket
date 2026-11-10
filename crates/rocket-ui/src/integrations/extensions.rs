//! Preferences → Extensions: the list of extensions, and the details of one. A connection is a
//! setting inside its extension; the Enable switch reads the connections' saved state.
use super::*;
use crate::{chrome::icon_button, folder_browser::PickTarget};

/// One enabled extension, for the sidebar, Preferences, the menus and the palette.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub id: String,
    pub name: String,
    pub letter: char,
    /// Waiting for a review, or for a check of a write that may have reached the service.
    pub attention: bool,
    /// The commands of the approved copy.
    pub commands: Vec<rocket_integrations::Command>,
    /// What keeps every command from running.
    pub block: Option<Fix>,
    /// The selected connection shares repository details.
    pub shares: bool,
}

/// The first letter or digit of a name, for the tile that stands in for an icon.
pub(crate) fn letter(name: &str) -> char {
    name.chars()
        .find(|c| c.is_alphanumeric())
        .map_or('?', |c| c.to_uppercase().next().unwrap_or(c))
}

/// A letter on a rounded tile. Packages have no icon, and adding one would change API 1.
pub(crate) fn letter_tile(painter: &Painter, rect: Rect, letter: char, ink: Color32) {
    painter.rect_filled(rect, (rect.height() * 0.24).round(), RAISED());
    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        letter,
        semibold((rect.height() * 0.46).round().max(11.0)),
        ink,
    );
}

/// Whether `candidate` is a later version. Dotted numbers compare as numbers; other text,
/// which a manifest may use, counts as later whenever it differs.
pub(super) fn later(candidate: &str, current: &str) -> bool {
    let parts = |v: &str| v.split('.').map(str::parse::<u64>).collect::<std::result::Result<Vec<_>, _>>();
    match (parts(candidate), parts(current)) {
        (Ok(candidate), Ok(current)) => candidate > current,
        _ => candidate != current,
    }
}

/// What the page asked for, done once the state is no longer borrowed.
#[derive(Default)]
pub(super) struct Asked {
    pub(super) pick: bool,
    pub(super) folder: Option<PathBuf>,
    pub(super) playground: bool,
}

impl State {
    /// Every extension to list: what is on disk and what ships inside Rocket, by name.
    fn listed(&self) -> Vec<&Package> {
        let mut list: Vec<&Package> = self.packages.iter().collect();
        list.extend(self.bundled.iter().filter(|b| !self.installed(&b.id)));
        list.sort_by(|a, b| a.name.cmp(&b.name));
        list
    }
    pub(super) fn is_bundled(&self, id: &str) -> bool {
        self.bundled.iter().any(|b| b.id == id)
    }
    /// A newer copy shipped with this Rocket than the one on disk. It never lands by itself.
    fn newer_bundled(&self, package: &Package) -> Option<&Package> {
        self.bundled.iter().find(|b| {
            b.id == package.id && self.installed(&b.id) && later(&b.version, &package.version) && !b.same_files(package)
        })
    }
    /// The second line of a row: version, where it came from and what it can reach.
    fn summary(&self, package: &Package) -> String {
        let mut parts = vec![package.version.clone()];
        if let Some(bundled) = self.bundled.iter().find(|b| b.id == package.id) {
            let changed = self.installed(&package.id) && package.version == bundled.version && !package.same_files(bundled);
            parts.push(if changed { crate::i18n::tr("Local copy") } else { crate::i18n::tr("Bundled") }.to_owned());
        }
        parts.push(if package.network { crate::i18n::tr("Network") } else { crate::i18n::tr("Offline") }.to_owned());
        if package.writes {
            parts.push(crate::i18n::tr("Writes").to_owned());
        }
        if package.repository {
            parts.push(crate::i18n::tr("Repository").to_owned());
        }
        parts.join(" · ")
    }
    /// What the extension may do on its connections, for one access switch.
    fn access(&self, key: &str) -> String {
        let all = self.connections();
        let on: Vec<&str> = all
            .iter()
            .filter(|(id, _)| profiles::connection(&self.store, &self.selected, id)[key] == true)
            .map(|(_, name)| name.as_str())
            .collect();
        let off: Vec<&str> = all.iter().map(|(_, name)| name.as_str()).filter(|n| !on.contains(n)).collect();
        match (on.len(), off.len()) {
            (0, 1) => crate::i18n::tr("Off").to_owned(),
            (0, 2) => crate::i18n::tr("Off for both connections").to_owned(),
            (0, _) => crate::i18n::tr("Off for all connections").to_owned(),
            (1, 0) => crate::i18n::tr("On").to_owned(),
            (_, 0) => crate::i18n::tr("On for all connections").to_owned(),
            (1, 1) => crate::i18n::trf!("On for {enabled} · Off for {disabled}", enabled = on[0], disabled = off[0]),
            (n, _) => crate::i18n::trf!("On for {count} of {total} connections", count = n, total = all.len()),
        }
    }
    /// The enabled extension whose details are open in Preferences.
    pub(crate) fn details_entry(&self) -> Option<String> {
        self.details.then(|| self.entries.iter().find(|e| e.id == self.selected).map(|e| e.id.clone())).flatten()
    }
    pub(crate) fn show_details(&mut self, backend: Backend, id: String) {
        self.details = true;
        self.perform(backend, Action::Package(id));
    }
    /// The sidebar or rail should scroll to the extension that was just opened.
    pub(crate) fn take_reveal(&mut self) -> bool {
        std::mem::take(&mut self.reveal_entry)
    }
    fn count_text(&self) -> String {
        let (on, all) = (self.entries.len(), self.listed().len());
        if on == 0 { crate::i18n::tr("None enabled").to_owned() } else { crate::i18n::trf!("{enabled} of {total} enabled", enabled = on, total = all) }
    }
    /// The details show a connection form that differs from what is saved, so the dialog's
    /// footer holds Save and Discard, in view however far the page has scrolled.
    pub(crate) fn form_open(&self) -> bool {
        self.details && self.installed(&self.selected) && self.draft != self.baseline
    }
    pub(crate) fn form_footer(&mut self, ui: &mut Ui, widgets: &mut HashMap<String, UiControl>, backend: Backend, writable: bool) {
        let idle = !self.running();
        let r = pages::primary(ui, idle && writable && !self.uncertain(), crate::i18n::tr("Save"));
        pages::control(widgets, "int-save", &r, ui.clip_rect());
        if r.clicked() {
            self.perform(backend, Action::Save);
        }
        if button(ui, widgets, "int-discard", crate::i18n::tr("Discard"), idle) {
            self.reset_draft();
        }
        ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
            let (rect, _) = ui.allocate_exact_size(vec2(8.0, 8.0), Sense::hover());
            ui.painter().circle_filled(rect.center(), 3.5, AMBER());
            ui.add(Label::new(RichText::new(crate::i18n::tr("Unsaved changes")).size(12.5).color(AMBER())).truncate());
        });
    }
}

/// A card's header bar. Controls added to it go from the right edge; end with [`bar_left`].
fn bar(ui: &mut Ui) -> Ui {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 40.0), Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius { nw: 7, ne: 7, sw: 0, se: 0 }, CARD());
    ui.painter().hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0, CARD_LINE()));
    let mut bar = ui.new_child(
        UiBuilder::new()
            .max_rect(rect.shrink2(vec2(14.0, 0.0)))
            .layout(Layout::right_to_left(Align::Center)),
    );
    bar.spacing_mut().item_spacing.x = 8.0;
    bar
}
/// The rest of a header bar, from its left edge.
fn bar_left(bar: &mut Ui, left: impl FnOnce(&mut Ui)) {
    bar.with_layout(Layout::left_to_right(Align::Center), left);
}
/// A card's foot: a row of 46 points under a line, laid out from the left.
fn foot(ui: &mut Ui, content: impl FnOnce(&mut Ui)) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 46.0), Sense::hover());
    ui.painter().hline(rect.x_range(), rect.top() + 0.5, Stroke::new(1.0, CARD_LINE()));
    let mut row = ui.new_child(
        UiBuilder::new()
            .max_rect(rect.shrink2(vec2(14.0, 0.0)))
            .layout(Layout::left_to_right(Align::Center)),
    );
    row.spacing_mut().item_spacing.x = 8.0;
    content(&mut row);
}

fn card_title(ui: &mut Ui, title: &str, detail: &str) {
    ui.add(Label::new(RichText::new(title).font(semibold(13.0))).truncate());
    if !detail.is_empty() {
        ui.add(Label::new(RichText::new(detail).size(12.0).color(FAINT())).truncate());
    }
}

struct Row<'a> {
    key: String,
    letter: char,
    ink: Color32,
    name: &'a str,
    detail: &'a str,
    last: bool,
    /// The row opens the extension's details.
    opens: bool,
}

/// A list row: letter tile, name and detail, then `trailing` laid out from the right edge.
fn list_row(
    ui: &mut Ui,
    widgets: &mut HashMap<String, UiControl>,
    row: Row,
    enabled: bool,
    trailing: impl FnOnce(&mut Ui, &mut HashMap<String, UiControl>),
) -> Response {
    let opens = row.opens && enabled;
    let sense = if opens { Sense::click() } else { Sense::hover() };
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 54.0), sense);
    if opens && (response.hovered() || response.has_focus()) {
        ui.painter().rect_filled(rect, 0.0, HOVER());
    }
    if !row.last {
        ui.painter().hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0, ROW_LINE()));
    }
    let tile = Rect::from_min_size(pos2(rect.left() + 14.0, rect.center().y - 15.0), vec2(30.0, 30.0));
    letter_tile(ui.painter(), tile, row.letter, row.ink);
    let mut side = ui.new_child(
        UiBuilder::new()
            .max_rect(rect.shrink2(vec2(10.0, 0.0)))
            .layout(Layout::right_to_left(Align::Center)),
    );
    side.spacing_mut().item_spacing.x = 10.0;
    if row.opens {
        let (chevron, _) = side.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
        icon(side.painter(), chevron.shrink(1.0), Icon::ChevronRight, FAINT());
    }
    trailing(&mut side, widgets);
    let right = side.min_rect().left() - 8.0;
    let left = tile.right() + 12.0;
    let room = (right - left).max(0.0);
    let painter = ui.painter();
    let mut job = text::LayoutJob::simple_singleline(row.name.into(), semibold(13.0), TEXT());
    job.wrap = text::TextWrapping::truncate_at_width(room);
    let name = painter.layout_job(job);
    painter.galley(pos2(left, rect.center().y - 9.0 - name.size().y / 2.0), name, TEXT());
    let mut job = text::LayoutJob::simple_singleline(row.detail.into(), FontId::proportional(11.5), FAINT());
    job.wrap = text::TextWrapping::truncate_at_width(room);
    let detail = painter.layout_job(job);
    painter.galley(pos2(left, rect.center().y + 9.0 - detail.size().y / 2.0), detail, FAINT());
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, enabled, row.name));
    pages::control(widgets, row.key, &response, ui.clip_rect());
    response
}

/// A row shape while the first read is under way, so the page doesn't start blank.
fn placeholder_row(ui: &mut Ui, name: f32, detail: f32, last: bool) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 54.0), Sense::hover());
    let painter = ui.painter();
    if !last {
        painter.hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0, ROW_LINE()));
    }
    let ink = RAISED();
    let left = rect.left() + 56.0;
    painter.rect_filled(Rect::from_min_size(pos2(rect.left() + 14.0, rect.center().y - 15.0), vec2(30.0, 30.0)), 7.0, ink);
    painter.rect_filled(Rect::from_min_size(pos2(left, rect.center().y - 13.0), vec2(name, 9.0)), 4.5, ink);
    painter.rect_filled(Rect::from_min_size(pos2(left, rect.center().y + 5.0), vec2(detail, 8.0)), 4.0, ink.gamma_multiply(0.7));
}

/// The list's rows: every extension, then the folders that couldn't load.
fn rows(ui: &mut Ui, s: &mut State, widgets: &mut HashMap<String, UiControl>, backend: &Backend, enabled: bool, writable: bool) {
    let mut action = None;
    let mut open = None;
    let listed: Vec<Package> = s.listed().into_iter().cloned().collect();
    let issues = s.package_issues.clone();
    for (i, package) in listed.iter().enumerate() {
        let on = profiles::extension_enabled(&s.store, &package.id);
        let review = on && s.installed(&package.id) && profiles::needs_review(&s.store, package);
        let update = s.newer_bundled(package).map(|b| b.version.clone());
        let detail = s.summary(package);
        let row = Row {
            key: format!("int-package-{}", package.id),
            letter: letter(&package.name),
            ink: if review { AMBER() } else { MUTED() },
            name: &package.name,
            detail: &detail,
            last: i + 1 == listed.len() && issues.is_empty(),
            opens: true,
        };
        let response = list_row(ui, widgets, row, enabled, |ui, widgets| {
            let mut switched = on;
            let r = ui.add_enabled_ui(enabled && writable, |ui| switch(ui, &mut switched, &package.name)).inner;
            pages::control(widgets, format!("int-enable-{}", package.id), &r, ui.clip_rect());
            if r.clicked() {
                action = Some(Action::Switch(package.id.clone(), switched));
            }
            if let Some(version) = &update {
                let r = ui.add_enabled(enabled && writable, Button::new(crate::i18n::trf!("Update to {version}", version)));
                pages::control(widgets, format!("int-update-{}", package.id), &r, ui.clip_rect());
                if r.clicked() {
                    action = Some(Action::Update(package.id.clone()));
                }
            }
            if review {
                if button(ui, widgets, &format!("int-review-{}", package.id), crate::i18n::tr("Review"), enabled && writable) {
                    action = Some(Action::Review(package.id.clone()));
                }
                pages::pill(ui, crate::i18n::tr("Files changed"), AMBER());
            }
        });
        if response.clicked() {
            open = Some(package.id.clone());
        }
        response.context_menu(|ui| {
            if ui.button(crate::i18n::tr("Configure")).clicked() {
                open = Some(package.id.clone());
                ui.close();
            }
            if update.is_some() && ui.button(crate::i18n::tr("Update")).clicked() {
                action = Some(Action::Update(package.id.clone()));
                ui.close();
            }
            if review && ui.button(crate::i18n::tr("Review")).clicked() {
                action = Some(Action::Review(package.id.clone()));
                ui.close();
            }
            if s.removable(package) && ui.button(crate::i18n::tr("Remove")).clicked() {
                s.ask_remove(package.clone());
                ui.close();
            }
        });
    }
    for (i, issue) in issues.iter().enumerate() {
        let code = issue.error.split(':').next().unwrap_or_default();
        let (detail, name) = match &issue.duplicate {
            Some((id, version)) => {
                let copies = issues.iter().filter(|o| o.duplicate.as_ref().is_some_and(|d| &d.0 == id)).count();
                (crate::i18n::trf!("ID {id} · {version} · {copies} folders use this ID; all stay off", id, version, copies), letter(id))
            }
            None => (shown(&issue.error).to_owned(), '!'),
        };
        let row = Row {
            key: format!("int-issue-{i}"),
            letter: name,
            ink: RED(),
            name: &issue.folder,
            detail: &detail,
            last: i + 1 == issues.len(),
            opens: false,
        };
        list_row(ui, widgets, row, enabled, |ui, widgets| {
            if issue.duplicate.is_some() {
                let r = pages::danger_ghost(ui, enabled && writable, crate::i18n::tr("Remove"));
                pages::control(widgets, format!("int-issue-remove-{i}"), &r, ui.clip_rect());
                if r.clicked() {
                    s.ask_remove_folder(issue.clone());
                }
                pages::pill(ui, crate::i18n::tr("Duplicate ID"), RED());
            } else {
                if quiet(ui, widgets, &format!("int-issue-folder-{i}"), crate::i18n::tr("Open folder"), true) {
                    s.reveal = Some(issue.directory.clone());
                }
                pages::pill(ui, if code == "integration_api" { crate::i18n::tr("Needs a newer Rocket") } else { crate::i18n::tr("Invalid") }, RED());
            }
        });
    }
    if let Some(id) = open {
        s.details = true;
        s.perform(backend.clone(), Action::Package(id));
    }
    if let Some(action) = action {
        s.perform(backend.clone(), action);
    }
}

fn list(ui: &mut Ui, s: &mut State, widgets: &mut HashMap<String, UiControl>, backend: &Backend, writable: bool, asked: &mut Asked) {
    let idle = !s.running();
    let enabled = idle && s.preview.is_none();
    preferences::card_frame(ui, |ui| {
        let mut strip = bar(ui);
        let r = pages::primary(&mut strip, idle && writable, crate::i18n::tr("Install from file"));
        pages::control(widgets, "int-install-file", &r, strip.clip_rect());
        asked.pick = r.clicked();
        let r = icon_button(&mut strip, Icon::Refresh, idle, false, 28.0).on_hover_text(crate::i18n::tr("Reload folders"));
        pages::control(widgets, "int-reload", &r, strip.clip_rect());
        if r.clicked() {
            s.perform(backend.clone(), Action::Reload(None));
        }
        let count = if s.listed_once { s.count_text() } else { String::new() };
        bar_left(&mut strip, |ui| card_title(ui, crate::i18n::tr("Extensions"), &count));
        if !s.listed_once {
            placeholder_row(ui, 120.0, 240.0, false);
            placeholder_row(ui, 170.0, 200.0, false);
            placeholder_row(ui, 140.0, 220.0, true);
            return;
        }
        if let Some(error) = s.list_error.clone() {
            preferences::card_note(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 10.0;
                    if button(ui, widgets, "int-reload-retry", crate::i18n::tr("Try again"), idle) {
                        s.perform(backend.clone(), Action::Reload(None));
                    }
                    ui.add(Label::new(RichText::new(shown(&error)).size(12.5).color(RED())).wrap());
                });
            });
            return;
        }
        rows(ui, s, widgets, backend, enabled, writable);
        if s.packages.iter().all(|p| s.is_bundled(&p.id)) && s.package_issues.is_empty() {
            ui.painter().hline(ui.max_rect().x_range(), ui.cursor().top() - 0.5, Stroke::new(1.0, ROW_LINE()));
            ui.add_space(26.0);
            ui.vertical_centered(|ui| {
                let (rect, _) = ui.allocate_exact_size(vec2(24.0, 24.0), Sense::hover());
                icon(ui.painter(), rect, Icon::Puzzle, FAINT());
                ui.add_space(8.0);
                ui.label(RichText::new(crate::i18n::tr("No other extensions installed")).color(MUTED()));
            });
            ui.add_space(28.0);
        }
    });
    notices(ui, s, widgets, backend, false);
    let r = pages::disclosure(ui, s.tools_open, crate::i18n::tr("Developer tools"));
    pages::control(widgets, "int-developer", &r, ui.clip_rect());
    if r.clicked() {
        s.tools_open = !s.tools_open;
    }
    if !s.tools_open {
        return;
    }
    preferences::card_frame(ui, |ui| {
        preferences::card_header(ui, crate::i18n::tr("Developer"), |_| {});
        let folder = s.directory.join("integrations");
        tool_row(ui, crate::i18n::tr("Folder"), &crate::views::display_path(&folder), true, false, |ui| {
            if button(ui, widgets, "int-open-folder", crate::i18n::tr("Open folder"), true) {
                asked.folder = Some(folder.clone());
            }
        });
        tool_row(ui, crate::i18n::tr("Lua playground"), crate::i18n::tr("Lua editor with a live preview"), false, false, |ui| {
            asked.playground = button(ui, widgets, "int-open-playground", crate::i18n::tr("Open Lua playground"), idle && writable);
        });
        tool_row(ui, crate::i18n::tr("Sample"), crate::i18n::tr("Repository context"), false, true, |ui| {
            if button(ui, widgets, "int-install-repository", crate::i18n::tr("Add sample"), idle && writable && !s.installed("repository")) {
                s.perform(backend.clone(), Action::Reload(Some("repository")));
            }
        });
    });
}

/// A row of the Developer card: a label, what it is, and its button at the right.
fn tool_row(ui: &mut Ui, label: &str, value: &str, mono: bool, last: bool, side: impl FnOnce(&mut Ui)) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 46.0), Sense::hover());
    if !last {
        ui.painter().hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0, CARD_LINE()));
    }
    let mut row = ui.new_child(
        UiBuilder::new()
            .max_rect(rect.shrink2(vec2(16.0, 0.0)))
            .layout(Layout::right_to_left(Align::Center)),
    );
    row.spacing_mut().item_spacing.x = 12.0;
    side(&mut row);
    row.with_layout(Layout::left_to_right(Align::Center), |ui| {
        let (label_rect, _) = ui.allocate_exact_size(vec2(100.0, 18.0), Sense::hover());
        ui.painter().text(label_rect.left_center(), Align2::LEFT_CENTER, label, FontId::proportional(13.0), MUTED());
        let text = RichText::new(value).size(12.5).color(TEXT_SOFT());
        ui.add(Label::new(if mono { text.monospace() } else { text }).truncate());
    });
}

/// One permission: its icon, its name, and what it means for this extension.
fn permission_row(ui: &mut Ui, image: Icon, name: &str, value: &str, off: bool) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 36.0), Sense::hover());
    ui.painter().hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0, ROW_LINE()));
    let painter = ui.painter();
    let (ink, soft) = if off { (FAINT(), FAINT()) } else { (TEXT(), MUTED()) };
    icon(painter, Rect::from_center_size(pos2(rect.left() + 22.0, rect.center().y), vec2(14.0, 14.0)), image, if off { FAINT() } else { HINT() });
    painter.text(pos2(rect.left() + 40.0, rect.center().y), Align2::LEFT_CENTER, name, FontId::proportional(13.0), ink);
    let left = rect.left() + 190.0;
    let mut job = text::LayoutJob::simple_singleline(value.into(), FontId::proportional(13.0), soft);
    job.wrap = text::TextWrapping::truncate_at_width((rect.right() - 14.0 - left).max(0.0));
    let galley = painter.layout_job(job);
    painter.galley(pos2(left, rect.center().y - galley.size().y / 2.0), galley, soft);
}

/// A small heading between the rows of a card.
fn card_section(ui: &mut Ui, title: &str) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::hover());
    ui.painter().hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0, ROW_LINE()));
    ui.painter().text(
        pos2(rect.left() + 16.0, rect.bottom() - 13.0),
        Align2::LEFT_CENTER,
        title.to_uppercase(),
        semibold(11.0),
        FAINT(),
    );
}

/// The back link above an extension's cards.
fn back_link(ui: &mut Ui, widgets: &mut HashMap<String, UiControl>, enabled: bool) -> bool {
    let label = crate::i18n::tr("All extensions");
    let galley = ui.painter().layout_no_wrap(label.to_owned(), FontId::proportional(13.0), MUTED());
    let (rect, response) = ui.allocate_exact_size(vec2(galley.size().x + 28.0, 26.0), Sense::click());
    let hot = enabled && (response.hovered() || response.has_focus());
    if hot {
        ui.painter().rect_filled(rect, 6.0, HOVER());
    }
    let ink = if hot { TEXT() } else { MUTED() };
    icon(ui.painter(), Rect::from_center_size(pos2(rect.left() + 10.0, rect.center().y), vec2(11.0, 11.0)), Icon::ArrowLeft, ink);
    ui.painter().galley(pos2(rect.left() + 22.0, rect.center().y - galley.size().y / 2.0), galley, ink);
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, enabled, label));
    pages::control(widgets, "int-back", &response, ui.clip_rect());
    enabled && response.clicked()
}

/// The fields of the selected connection: service, settings and access, then its foot.
fn connection_rows(ui: &mut Ui, s: &mut State, widgets: &mut HashMap<String, UiControl>, package: &Package, idle: bool, writable: bool) {
    ui.add_enabled_ui(idle, |ui| {
        if package.network {
            preferences::field_row(ui, crate::i18n::tr("Name"), |ui| edit(ui, widgets, "int-connection-name", &mut s.draft.name, false, "Default"));
            preferences::divider(ui);
            preferences::field_row(ui, crate::i18n::tr("Server"), |ui| edit(ui, widgets, "int-server", &mut s.draft.origin, false, "https://"));
            preferences::divider(ui);
            preferences::field_row(ui, crate::i18n::tr("Authentication"), |ui| {
                let modes = [("none", crate::i18n::tr("None")), ("bearer", crate::i18n::tr("Bearer token")), ("basic", crate::i18n::tr("Identifier + token"))];
                let items: Vec<(&str, bool)> = modes.iter().map(|(key, label)| (*label, s.draft.auth == *key)).collect();
                let ids: Vec<String> = modes.iter().map(|(key, _)| format!("int-auth-{key}")).collect();
                ui.horizontal(|ui| {
                    if let Some(i) = preferences::choices(ui, widgets, &items, &ids) {
                        s.draft.auth = modes[i].0.into();
                    }
                });
            });
            if s.draft.auth == "basic" {
                preferences::divider(ui);
                preferences::field_row(ui, crate::i18n::tr("Identifier"), |ui| edit(ui, widgets, "int-username", &mut s.draft.username, false, ""));
            }
            if s.draft.auth != "none" {
                preferences::divider(ui);
                preferences::field_row(ui, crate::i18n::tr("API token"), |ui| {
                    edit(ui, widgets, "int-token", &mut s.draft.token, true, crate::i18n::tr("Kept by Rocket, never given to Lua"));
                    let r = ui.add(checkbox(&mut s.draft.remember, crate::i18n::tr("Remember on this device")));
                    pages::control(widgets, "int-remember", &r, ui.clip_rect());
                });
            }
        }
        if !package.settings.is_empty() {
            if package.network {
                card_section(ui, crate::i18n::tr("Settings"));
            }
            for (i, setting) in package.settings.iter().enumerate() {
                if i > 0 {
                    preferences::divider(ui);
                }
                let mut value = s.draft.settings.get(&setting.id).and_then(Value::as_str).unwrap_or("").to_owned();
                preferences::field_row(ui, &setting.label, |ui| edit(ui, widgets, &format!("int-setting-{}", setting.id), &mut value, false, ""));
                s.draft.settings.insert(setting.id.clone(), json!(value));
            }
        }
        if package.writes || package.repository {
            if package.network {
                card_section(ui, crate::i18n::tr("Access"));
            } else if !package.settings.is_empty() {
                preferences::divider(ui);
            }
            if package.writes {
                preferences::switch_row(ui, widgets, "int-allow-write", &mut s.draft.writes, crate::i18n::tr("Allow writes"), crate::i18n::tr("Rocket shows each write before it is sent"), "");
            }
            if package.repository {
                if package.writes {
                    preferences::divider(ui);
                }
                preferences::switch_row(ui, widgets, "int-allow-repository", &mut s.draft.repository,
                    crate::i18n::tr("Share branch, commit IDs and change counts"), crate::i18n::tr("Never paths, remote URLs, identities or file contents"), "");
            }
        }
    });
    if package.network {
        foot(ui, |ui| {
            let r = pages::danger_ghost(ui, idle && writable && !s.uncertain(), crate::i18n::tr("Remove connection"));
            pages::control(widgets, "int-remove-connection", &r, ui.clip_rect());
            if r.clicked() {
                s.remove_confirm = true;
            }
        });
    }
}

fn details(ui: &mut Ui, s: &mut State, widgets: &mut HashMap<String, UiControl>, backend: &Backend, writable: bool) {
    let Some(package) = s.package().cloned() else {
        return;
    };
    let idle = !s.running();
    let installed = s.installed(&package.id);
    if back_link(ui, widgets, idle) {
        s.details = false;
        s.error = None;
        return;
    }
    let on = profiles::extension_enabled(&s.store, &package.id);
    let review = on && installed && profiles::needs_review(&s.store, &package);
    let update = s.newer_bundled(&package).map(|b| b.version.clone());
    let bundled = s.is_bundled(&package.id);
    preferences::card_frame(ui, |ui| {
        let mut strip = bar(ui);
        let mut switched = on;
        let r = strip.add_enabled_ui(idle && writable, |ui| switch(ui, &mut switched, &package.name)).inner;
        pages::control(widgets, "int-enable", &r, strip.clip_rect());
        if on {
            pages::control(widgets, "int-disable", &r, strip.clip_rect());
        }
        if r.clicked() {
            s.perform(backend.clone(), Action::Switch(package.id.clone(), switched));
        }
        strip.label(RichText::new(crate::i18n::tr("Enabled")).size(12.0).color(HINT()));
        bar_left(&mut strip, |ui| {
            let (tile, _) = ui.allocate_exact_size(vec2(24.0, 24.0), Sense::hover());
            letter_tile(ui.painter(), tile, letter(&package.name), BLUE());
            let detail = if bundled {
                crate::i18n::trf!("{version} · Bundled · Lua API 1", version = package.version)
            } else {
                format!("{} · Lua API 1", package.version)
            };
            card_title(ui, &package.name, &detail);
        });
        if !package.description.is_empty() {
            Frame::new().inner_margin(Margin::symmetric(14, 11)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.add(Label::new(RichText::new(&package.description).color(MUTED())).wrap());
            });
            preferences::divider(ui);
        }
        let not = crate::i18n::tr("Not requested");
        permission_row(ui, Icon::Remote, crate::i18n::tr("Network"), if package.network { crate::i18n::tr("Only the server set in each connection") } else { not }, !package.network);
        let writes = if package.writes { s.access("writes") } else { not.to_owned() };
        permission_row(ui, Icon::Pencil, crate::i18n::tr("Reviewed writes"), &writes, !package.writes);
        let repository = if package.repository { s.access("repository") } else { not.to_owned() };
        permission_row(ui, Icon::Branch, crate::i18n::tr("Repository details"), &repository, !package.repository);
        foot(ui, |ui| {
            if s.removable(&package) {
                let r = pages::danger_ghost(ui, idle && writable, crate::i18n::tr("Remove"));
                pages::control(widgets, "int-remove", &r, ui.clip_rect());
                if r.clicked() {
                    s.ask_remove(package.clone());
                }
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                if let Some(version) = &update {
                    let r = pages::primary(ui, idle && writable, &crate::i18n::trf!("Update to {version}", version));
                    pages::control(widgets, "int-update", &r, ui.clip_rect());
                    if r.clicked() {
                        s.perform(backend.clone(), Action::Update(package.id.clone()));
                    }
                    ui.label(RichText::new(crate::i18n::trf!("Rocket includes {version}", version)).size(12.0).color(HINT()));
                }
                if installed && !bundled && button(ui, widgets, "int-update-file", crate::i18n::tr("Update from file"), idle && writable) {
                    s.pick = true;
                }
                if review {
                    if button(ui, widgets, "int-review", crate::i18n::tr("Review"), idle && writable) {
                        s.perform(backend.clone(), Action::Review(package.id.clone()));
                    }
                    pages::pill(ui, crate::i18n::tr("Files changed"), AMBER());
                }
            });
        });
    });
    // A bundled extension has no connection until its first Enable puts it on disk.
    if installed {
        if package.network {
            preferences::card_frame(ui, |ui| {
                let mut strip = bar(ui);
                if quiet(&mut strip, widgets, "int-new-connection", crate::i18n::tr("New connection"), idle && writable) {
                    s.perform(backend.clone(), Action::New);
                }
                bar_left(&mut strip, |ui| {
                    card_title(ui, crate::i18n::tr("Connection"), "");
                    ui.add_space(4.0);
                    choose_connection(ui, s, widgets, idle && s.preview.is_none(), backend, false);
                });
                connection_rows(ui, s, widgets, &package, idle, writable);
            });
        } else if !package.settings.is_empty() || package.repository || package.writes {
            preferences::card_frame(ui, |ui| {
                let mut strip = bar(ui);
                bar_left(&mut strip, |ui| {
                    card_title(ui, crate::i18n::tr("Settings"), "");
                    if s.connections().len() > 1 {
                        ui.add_space(4.0);
                        choose_connection(ui, s, widgets, idle && s.preview.is_none(), backend, false);
                    }
                });
                connection_rows(ui, s, widgets, &package, idle, writable);
            });
        }
    }
    // Below the connection, which is the next step after Enable.
    if !package.commands.is_empty() {
        preferences::card_frame(ui, |ui| {
            let mut strip = bar(ui);
            bar_left(&mut strip, |ui| card_title(ui, crate::i18n::tr("Commands"), ""));
            let shares = profiles::connection(&s.store, &package.id, profiles::selected(&s.store, &package.id))["repository"] == true;
            for command in &package.commands {
                let needs = command.target.is_some() && !shares;
                review::command_row(ui, command, needs.then_some(crate::i18n::tr("Needs repository details")));
            }
        });
    }
    if s.remove_confirm {
        pages::callout(
            ui,
            AMBER(),
            &crate::i18n::trf!(
                "Remove {connection} and its saved draft and credential from this device? Remote data is unchanged.",
                connection = s.profile()["name"].as_str().unwrap_or("Default")
            ),
        );
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            if button(ui, widgets, "int-remove-back", crate::i18n::tr("Keep connection"), idle) {
                s.remove_confirm = false;
            }
            let r = pages::danger(ui, idle && !s.uncertain(), crate::i18n::tr("Remove this connection"));
            pages::control(widgets, "int-remove-confirm", &r, ui.clip_rect());
            if r.clicked() {
                s.perform(backend.clone(), Action::Remove);
            }
        });
    }
    notices(ui, s, widgets, backend, false);
    developer_panel(ui, s, widgets, backend);
}

impl Rocket {
    /// Preferences → Extensions: the list, or the extension chosen from it. Returns the picker
    /// to open, when Install from file or Update from file was chosen.
    pub(crate) fn integration_settings(&mut self, ui: &mut Ui) -> Option<PickTarget> {
        let Some(backend) = self.backend.clone() else {
            ui.label(crate::i18n::tr("Waiting for Rocket to start."));
            return None;
        };
        let writable = self.profile_storage_ready();
        let s = &mut self.integrations;
        let widgets = &mut self.frame_widgets;
        let mut asked = Asked::default();
        ui.spacing_mut().item_spacing.y = 10.0;
        if s.details && s.package().is_some() {
            details(ui, s, widgets, &backend, writable);
        } else {
            list(ui, s, widgets, &backend, writable, &mut asked);
        }
        asked.pick |= std::mem::take(&mut s.pick);
        if let Some(folder) = asked.folder.take().or_else(|| s.reveal.take()) {
            self.show_folder(&folder);
        }
        if asked.playground {
            self.open_playground();
        }
        asked.pick.then_some(PickTarget::ExtensionFile)
    }
    /// The Lua playground, in the content area where it has the room.
    pub(crate) fn open_playground(&mut self) {
        let Some(backend) = self.backend.clone() else {
            return;
        };
        let s = &mut self.integrations;
        if s.playground.loaded() {
            s.playground.open = true;
        } else {
            s.playground.load(backend, s.directory.clone());
        }
        self.dispatch(Intent::Dialog(DialogKind::Integrations));
    }
    /// A sidebar, menu or palette entry: that extension's page.
    pub(crate) fn open_extension(&mut self, id: String) {
        let Some(backend) = self.backend.clone() else {
            return;
        };
        self.integrations.playground.open = false;
        self.integrations.reveal_entry = true;
        self.integrations.perform(backend, Action::Package(id));
        self.dispatch(Intent::Dialog(DialogKind::Integrations));
    }
    /// Preferences at one extension's details.
    pub(crate) fn configure_extension(&mut self) {
        self.show_preferences(preferences::Page::Integrations);
        self.integrations.details = true;
    }
}
