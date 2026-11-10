//! The reviews around an extension: Enable, Install, Update, Review again and Remove. One dialog
//! shows where it came from, its code, its permissions and what stays. No review runs a script.
use super::*;
use crate::{folder_browser::PickTarget, pages::Part};
use rocket_integrations::{ApprovedCopy, Command, CommandChange, CommandLine, Line, LineKind, SourceChange, Target, compare_commands, compare_source};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Kind {
    Enable,
    Install,
    Update,
    Again,
}
struct Permission {
    image: Icon,
    name: &'static str,
    value: String,
    new: bool,
    off: bool,
}
pub(super) struct Review {
    pub(super) kind: Kind,
    pub(super) package: Package,
    /// The copy on disk that this one replaces.
    replaces: Option<Package>,
    /// The package is not on disk yet, or is replacing what is.
    copies: bool,
    file: Option<PathBuf>,
    version: String,
    change: Option<SourceChange>,
    permissions: [Permission; 3],
    /// New, changed and removed commands, then the ones that stay.
    commands: Vec<CommandLine>,
    /// Compared with an approved copy, so unchanged commands fold into one line.
    compared: bool,
    kept: Vec<String>,
    drafts: usize,
    /// A connection whose last write may have reached the service.
    blocked: Option<String>,
    code: bool,
}
pub(super) struct Removal {
    title: String,
    id: Option<String>,
    directory: Option<PathBuf>,
    rows: Vec<(&'static str, String)>,
    blocked: Option<String>,
}
pub(super) enum Prompt {
    Review(Box<Review>),
    /// The chosen file can't be installed.
    Failed { file: PathBuf, error: String },
    /// The chosen file is what is already installed.
    Installed { file: PathBuf, id: String, name: String, version: String },
    Remove(Removal),
}

/// "Default", "Default and Work", or "Default, Work and Staging".
fn names(list: &[String]) -> String {
    match list {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => crate::i18n::trf!("{names} and {last}", names = rest.join(", "), last),
    }
}

impl State {
    pub(super) fn installed(&self, id: &str) -> bool {
        self.packages.iter().any(|p| p.id == id)
    }
    pub(crate) fn prompt_open(&self) -> bool {
        self.prompt.is_some()
    }
    /// Something can be taken away: files on disk, or connections saved for this ID.
    pub(super) fn removable(&self, package: &Package) -> bool {
        self.installed(&package.id) || !self.store["profiles"][&package.id].is_null()
    }
    fn permissions(&self, package: &Package, previous: Option<&ApprovedCopy>) -> [Permission; 3] {
        let granted = |key: &str| {
            let list = &self.store["profiles"][&package.id]["connections"];
            list.as_object()
                .into_iter()
                .flatten()
                .filter(|(_, c)| c[key] == true)
                .map(|(_, c)| c["name"].as_str().unwrap_or("Default").to_owned())
                .collect::<Vec<_>>()
        };
        let line = |image, name, asks: bool, before: Option<bool>, first: &str, key: &str| {
            let (value, new, off) = match (asks, before) {
                (false, Some(true)) => (crate::i18n::tr("Removed").to_owned(), false, true),
                (false, _) => (crate::i18n::tr("Not requested").to_owned(), false, true),
                (true, Some(true)) => {
                    let on = granted(key);
                    let value = if on.is_empty() { crate::i18n::tr("No change").to_owned() } else { crate::i18n::trf!("No change · on for {connections}", connections = names(&on)) };
                    (value, false, false)
                }
                (true, Some(false)) => (first.to_owned(), true, false),
                (true, None) => (first.to_owned(), false, false),
            };
            Permission { image, name, value, new, off }
        };
        let off = crate::i18n::tr("Off until you allow it");
        [
            line(Icon::Remote, crate::i18n::mark("Network"), package.network, previous.map(|p| p.network), crate::i18n::tr("Only the server you set"), ""),
            line(Icon::Pencil, crate::i18n::mark("Reviewed writes"), package.writes, previous.map(|p| p.writes), off, "writes"),
            line(Icon::Branch, crate::i18n::mark("Repository details"), package.repository, previous.map(|p| p.repository), off, "repository"),
        ]
    }
    /// Opens a review of `package`, with everything it shows read now.
    fn open_review(&mut self, kind: Kind, package: Package, replaces: Option<Package>, file: Option<PathBuf>) {
        let id = package.id.clone();
        let previous = profiles::approved_copy(&self.store, &id).and_then(ApprovedCopy::read).or_else(|| {
            replaces.as_ref().map(|p| ApprovedCopy {
                name: p.name.clone(),
                version: p.version.clone(),
                network: p.network,
                writes: p.writes,
                repository: p.repository,
                commands: p.commands.clone(),
                source: p.source.clone(),
            })
        });
        let version = match &previous {
            Some(old) if old.version != package.version => format!("{} → {}", old.version, package.version),
            _ => package.version.clone(),
        };
        let compare = matches!(kind, Kind::Update | Kind::Again);
        let change = compare.then(|| previous.as_ref().map(|p| compare_source(&p.source, &package.source))).flatten();
        let commands = match previous.as_ref().filter(|_| compare) {
            Some(old) => compare_commands(&old.commands, &package.commands),
            None => compare_commands(&package.commands, &package.commands),
        };
        let review = Review {
            kind,
            permissions: self.permissions(&package, previous.as_ref().filter(|_| compare)),
            commands,
            compared: compare,
            copies: matches!(kind, Kind::Install | Kind::Update) || !self.installed(&id),
            replaces,
            file,
            version,
            change,
            kept: profiles::saved_connections(&self.store, &id),
            drafts: profiles::draft_count(&self.store, &id),
            blocked: matches!(kind, Kind::Update).then(|| profiles::unchecked_write(&self.store, &id)).flatten(),
            code: false,
            package,
        };
        self.error = None;
        self.prompt = Some(Prompt::Review(Box::new(review)));
    }
    /// Enable when these files were never approved, or the Review button.
    pub(super) fn ask_approval(&mut self, id: &str) {
        let Some(package) = self.packages.iter().chain(&self.bundled).find(|p| p.id == id).cloned() else {
            return;
        };
        let seen = profiles::approved_copy(&self.store, id).is_some();
        self.open_review(if seen { Kind::Again } else { Kind::Enable }, package, None, None);
    }
    /// Update to the copy that ships with this Rocket.
    pub(super) fn ask_update(&mut self, id: &str) {
        let installed = self.packages.iter().find(|p| p.id == id).cloned();
        let bundled = self.bundled.iter().find(|b| b.id == id).cloned();
        if let (Some(installed), Some(bundled)) = (installed, bundled) {
            self.open_review(Kind::Update, bundled, Some(installed), None);
        }
    }
    /// A file was read: its review, or why it can't be installed.
    pub(super) fn picked(&mut self, file: PathBuf, package: std::result::Result<Package, String>) {
        let package = match package {
            Ok(package) => package,
            Err(error) => {
                self.prompt = Some(Prompt::Failed { file, error });
                return;
            }
        };
        let failed = |error: String| Prompt::Failed { file: file.clone(), error };
        if let Some(bundled) = self.bundled.iter().find(|b| b.id == package.id) {
            let error = format!("integration_bundled: The ID {} belongs to the bundled {}. Give this package its own ID.", package.id, bundled.name);
            self.prompt = Some(failed(error));
        } else if self.package_issues.iter().any(|i| i.duplicate.as_ref().is_some_and(|d| d.0 == package.id)) {
            let error = format!("integration_duplicate: Folders already share the ID {}. Remove a copy first.", package.id);
            self.prompt = Some(failed(error));
        } else if let Some(old) = self.packages.iter().find(|p| p.id == package.id).cloned() {
            if old.same_files(&package) {
                self.prompt = Some(Prompt::Installed { file, id: old.id, name: old.name, version: old.version });
            } else {
                self.open_review(Kind::Update, package, Some(old), Some(file));
            }
        } else {
            self.open_review(Kind::Install, package, None, Some(file));
        }
    }
    pub(super) fn ask_remove(&mut self, package: Package) {
        let id = &package.id;
        let saved = profiles::saved_connections(&self.store, id);
        let drafts = profiles::draft_count(&self.store, id);
        let mut rows = vec![(crate::i18n::tr("Removed"), crate::i18n::tr("The installed copy").to_owned())];
        rows.push((crate::i18n::tr("Connections"), match saved.len() {
            0 => crate::i18n::tr("None").to_owned(),
            _ => crate::i18n::trf!("{connections}, including saved tokens", connections = names(&saved)),
        }));
        if drafts > 0 {
            rows.push((crate::i18n::tr("Drafts"), crate::i18n::trf!("Saved drafts: {count}", count = drafts)));
        }
        if self.is_bundled(id) {
            rows.push((crate::i18n::tr("List"), crate::i18n::trf!("{extension} stays here, off, because it ships with Rocket", extension = package.name)));
        }
        rows.push((crate::i18n::tr("Unchanged"), if package.network { crate::i18n::tr("Data on the service and your repositories").to_owned() } else { crate::i18n::tr("Your repositories").to_owned() }));
        self.error = None;
        self.prompt = Some(Prompt::Remove(Removal {
            title: crate::i18n::trf!("Remove {name}", name = package.name),
            blocked: profiles::unchecked_write(&self.store, id),
            directory: self.installed(id).then(|| package.directory.clone()),
            id: Some(package.id),
            rows,
        }));
    }
    /// One copy of a duplicate ID: only its folder goes.
    pub(super) fn ask_remove_folder(&mut self, issue: PackageIssue) {
        let others: Vec<String> = self
            .package_issues
            .iter()
            .filter(|o| o.directory != issue.directory && o.duplicate.as_ref().map(|d| &d.0) == issue.duplicate.as_ref().map(|d| &d.0))
            .map(|o| crate::i18n::trf!("The folder {folder}", folder = o.folder))
            .collect();
        let rows = vec![
            (crate::i18n::tr("Removed"), crate::i18n::trf!("The folder {folder}", folder = issue.folder)),
            (crate::i18n::tr("Connections"), crate::i18n::tr("Stay, for the copy that’s left").to_owned()),
            (crate::i18n::tr("Unchanged"), others.join(", ")),
        ];
        self.error = None;
        self.prompt = Some(Prompt::Remove(Removal {
            title: crate::i18n::trf!("Remove {name}", name = issue.folder),
            id: None,
            directory: Some(issue.directory),
            rows,
            blocked: None,
        }));
    }
    /// The review's main button: files first, then the store, so a failed save leaves new files
    /// unapproved rather than approval without files.
    fn confirm(&mut self, backend: Backend) {
        let Some(Prompt::Review(review)) = &self.prompt else {
            return;
        };
        let (package, replaces, copies) = (review.package.clone(), review.replaces.clone(), review.copies);
        // An update leaves an extension that was switched off as it was.
        let stay_off = review.kind == Kind::Update && !profiles::extension_enabled(&self.store, &package.id);
        let id = package.id.clone();
        let (directory, expected) = (self.directory.clone(), self.raw.clone());
        let mut value = self.store.clone();
        self.start("Approve extension", move |cancel| {
            backend.integration("integration-install", |_| {
                cancel.check()?;
                let root = directory.join("integrations");
                let landed = if copies {
                    rocket_integrations::install_package(&root, &package, replaces.as_ref())?
                } else {
                    package.verify()?;
                    package
                };
                profiles::approve(&mut value, &landed);
                if stay_off {
                    profiles::disable(&mut value, &id);
                }
                let raw = storage::save(&directory, &value, expected.as_deref())?;
                let catalog = rocket_integrations::discover(&root, &cancel)?;
                Ok(Done::Changed(catalog, value, Some(raw), Some(id)))
            })
        });
    }
    fn confirm_removal(&mut self, backend: Backend) {
        let Some(Prompt::Remove(removal)) = &self.prompt else {
            return;
        };
        let (id, folder) = (removal.id.clone(), removal.directory.clone());
        let (directory, expected) = (self.directory.clone(), self.raw.clone());
        let mut value = self.store.clone();
        self.start("Remove extension", move |cancel| {
            backend.integration("integration-remove", |_| {
                cancel.check()?;
                let root = directory.join("integrations");
                if let Some(folder) = folder {
                    rocket_integrations::remove_package(&root, &folder)?;
                }
                let mut raw = expected.clone();
                if let Some(id) = &id {
                    profiles::remove(&mut value, id);
                    raw = Some(storage::save(&directory, &value, expected.as_deref())?);
                }
                let catalog = rocket_integrations::discover(&root, &cancel)?;
                Ok(Done::Changed(catalog, value, raw, None))
            })
        });
    }
    /// A chosen `integration.json`: read off the UI thread; nothing is copied yet.
    pub(crate) fn pick_file(&mut self, backend: Backend, file: PathBuf) {
        self.prompt = None;
        self.start("Read package", move |cancel| {
            backend.integration("integration-read", |_| {
                cancel.check()?;
                let package = rocket_integrations::read_package_file(&file).map_err(|e| user_error(&e));
                Ok(Done::Picked(file, package))
            })
        });
    }
    /// What the dialog's owner should record in automation state.
    pub(super) fn prompt_snapshot(&self) -> Value {
        match &self.prompt {
            None => Value::Null,
            Some(Prompt::Review(r)) => json!({
                "kind": match r.kind { Kind::Enable => "enable", Kind::Install => "install", Kind::Update => "update", Kind::Again => "again" },
                "id": r.package.id,
                "version": r.version,
                "blocked": r.blocked.is_some(),
                "new_permissions": r.permissions.iter().filter(|p| p.new).map(|p| p.name).collect::<Vec<_>>(),
                "new_commands": r.commands.iter().filter(|c| c.change == CommandChange::New).map(|c| &c.command.id).collect::<Vec<_>>(),
                "changed_commands": r.commands.iter().filter(|c| matches!(c.change, CommandChange::Changed { .. })).map(|c| &c.command.id).collect::<Vec<_>>(),
                "removed_commands": r.commands.iter().filter(|c| c.change == CommandChange::Removed).map(|c| &c.command.id).collect::<Vec<_>>(),
                "added": r.change.as_ref().map(|c| c.added),
                "removed": r.change.as_ref().map(|c| c.removed),
                "drafts": r.drafts,
                "code_open": r.code,
            }),
            Some(Prompt::Failed { error, .. }) => json!({"kind":"failed","code":error.split(':').next().unwrap_or("integration_error")}),
            Some(Prompt::Installed { id, .. }) => json!({"kind":"installed","id":id}),
            Some(Prompt::Remove(r)) => json!({"kind":"remove","id":r.id,"blocked":r.blocked.is_some()}),
        }
    }
}

/// A sunken row: its label in a fixed column, the value, and a quiet action at the right.
fn well_row(ui: &mut Ui, label: &str, value: impl FnOnce(&mut Ui), side: impl FnOnce(&mut Ui)) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::hover());
    let mut row = ui.new_child(UiBuilder::new().max_rect(rect).layout(Layout::right_to_left(Align::Center)));
    row.spacing_mut().item_spacing.x = 8.0;
    side(&mut row);
    row.with_layout(Layout::left_to_right(Align::Center), |ui| {
        let (rect, _) = ui.allocate_exact_size(vec2(96.0, 18.0), Sense::hover());
        ui.painter().text(rect.left_center(), Align2::LEFT_CENTER, label, FontId::proportional(13.0), HINT());
        value(ui);
    });
}
fn plain(text: &str) -> impl FnOnce(&mut Ui) + '_ {
    move |ui| {
        ui.add(Label::new(RichText::new(text).size(13.0)).truncate());
    }
}
fn path_text(path: &std::path::Path) -> impl FnOnce(&mut Ui) {
    let text = crate::views::display_path(path);
    move |ui| {
        ui.add(Label::new(RichText::new(&text).monospace().size(12.0).color(TEXT_SOFT())).truncate());
    }
}
fn permissions(ui: &mut Ui, review: &Review) {
    pages::caption(ui, crate::i18n::tr("Permissions"));
    Frame::new()
        .fill(PANEL())
        .stroke(Stroke::new(1.0, CARD_STROKE()))
        .corner_radius(8)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            let never = [(Icon::Lock, crate::i18n::mark("Never"), crate::i18n::tr("Git or shell commands, your files, your tokens").to_owned(), false, false)];
            let show_never = matches!(review.kind, Kind::Enable | Kind::Install);
            let lines = review
                .permissions
                .iter()
                .map(|p| (p.image, p.name, p.value.clone(), p.new, p.off))
                .chain(never.into_iter().filter(|_| show_never));
            let count = 3 + usize::from(show_never);
            for (i, (image, name, value, new, off)) in lines.enumerate() {
                let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::hover());
                if i + 1 < count {
                    ui.painter().hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0, ROW_LINE()));
                }
                let (ink, soft) = if off { (FAINT(), FAINT()) } else { (TEXT(), MUTED()) };
                icon(ui.painter(), Rect::from_center_size(pos2(rect.left() + 20.0, rect.center().y), vec2(14.0, 14.0)), image, if off { FAINT() } else { HINT() });
                ui.painter().text(pos2(rect.left() + 38.0, rect.center().y), Align2::LEFT_CENTER, crate::i18n::tr(name), FontId::proportional(13.0), ink);
                let mut right = rect.right() - 12.0;
                if new {
                    let mut side = ui.new_child(UiBuilder::new().max_rect(rect.shrink2(vec2(12.0, 0.0))).layout(Layout::right_to_left(Align::Center)));
                    pages::pill(&mut side, crate::i18n::tr("New"), AMBER());
                    right = side.min_rect().left() - 8.0;
                }
                let left = rect.left() + 178.0;
                let mut job = text::LayoutJob::simple_singleline(value, FontId::proportional(13.0), soft);
                job.wrap = text::TextWrapping::truncate_at_width((right - left).max(0.0));
                let galley = ui.painter().layout_job(job);
                ui.painter().galley(pos2(left, rect.center().y - galley.size().y / 2.0), galley, soft);
            }
        });
}
/// Where a command shows and what it gets, in the words of the review and the details.
fn place(target: Option<Target>) -> &'static str {
    match target {
        None => crate::i18n::tr("Repository menu · Palette"),
        Some(Target::Commit) => crate::i18n::tr("Right-click a commit · Palette · Gets its ID"),
        Some(Target::Branch) => crate::i18n::tr("Right-click a branch · Palette · Gets its name"),
    }
}
/// One command on two lines: its label, then where it shows. `side` is amber text at the
/// right of the row, for a word that needs noticing.
pub(super) fn command_row(ui: &mut Ui, command: &Command, side: Option<&str>) {
    command_line(ui, command, place(command.target), false, |ui| {
        if let Some(side) = side {
            ui.label(RichText::new(side).size(12.0).color(AMBER()));
        }
    });
}
fn command_line(ui: &mut Ui, command: &Command, detail: &str, gone: bool, side: impl FnOnce(&mut Ui)) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 44.0), Sense::hover());
    ui.painter().hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0, ROW_LINE()));
    let image = match command.target {
        None => Icon::Folder,
        Some(Target::Commit) => Icon::Commit,
        Some(Target::Branch) => Icon::Branch,
    };
    icon(ui.painter(), Rect::from_center_size(pos2(rect.left() + 20.0, rect.center().y), vec2(14.0, 14.0)), image, if gone { FAINT() } else { HINT() });
    let mut trailing = ui.new_child(UiBuilder::new().max_rect(rect.shrink2(vec2(12.0, 0.0))).layout(Layout::right_to_left(Align::Center)));
    side(&mut trailing);
    let right = trailing.min_rect().left() - 8.0;
    let left = rect.left() + 38.0;
    let room = (right - left).max(0.0);
    for (words, y, font, ink) in [
        (command_label(&command.label), rect.center().y - 9.0, FontId::proportional(13.0), if gone { FAINT() } else { TEXT() }),
        (detail, rect.center().y + 9.0, FontId::proportional(11.5), FAINT()),
    ] {
        let mut job = text::LayoutJob::simple_singleline(words.to_owned(), font, ink);
        job.wrap = text::TextWrapping::truncate_at_width(room);
        let galley = ui.painter().layout_job(job);
        ui.painter().galley(pos2(left, y - galley.size().y / 2.0), galley, ink);
    }
}
/// What a changed command used to be, after where it shows now.
pub(super) fn was(label: &Option<String>, target: &Option<Option<Target>>) -> String {
    let at = |target: &Option<Target>| match target {
        None => crate::i18n::tr("in the Repository menu"),
        Some(Target::Commit) => crate::i18n::tr("on commits"),
        Some(Target::Branch) => crate::i18n::tr("on branches"),
    };
    match (label, target) {
        (Some(label), Some(target)) => crate::i18n::trf!(" · Was “{label}” {place}", label, place = at(target)),
        (Some(label), None) => crate::i18n::trf!(" · Was “{label}”", label),
        (None, Some(target)) => crate::i18n::trf!(" · Was {place}", place = at(target)),
        (None, None) => String::new(),
    }
}
/// The review's Commands card. A comparison leads with what changed and folds the rest.
fn commands(ui: &mut Ui, review: &Review) {
    if review.commands.is_empty() {
        return;
    }
    pages::caption(ui, crate::i18n::tr("Commands"));
    Frame::new()
        .fill(PANEL())
        .stroke(Stroke::new(1.0, CARD_STROKE()))
        .corner_radius(8)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            let mut same = 0;
            for line in &review.commands {
                let command = &line.command;
                match &line.change {
                    CommandChange::Same if review.compared => same += 1,
                    CommandChange::Same => command_row(ui, command, None),
                    CommandChange::New => command_line(ui, command, place(command.target), false, |ui| {
                        pages::pill(ui, crate::i18n::tr("New"), AMBER());
                    }),
                    CommandChange::Changed { label, target } => {
                        command_line(ui, command, &format!("{}{}", place(command.target), was(label, target)), false, |ui| {
                            pages::pill(ui, crate::i18n::tr("Changed"), AMBER());
                        });
                    }
                    CommandChange::Removed => command_line(ui, command, crate::i18n::tr("Removed"), true, |_| {}),
                }
            }
            if same > 0 {
                let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::hover());
                icon(ui.painter(), Rect::from_center_size(pos2(rect.left() + 20.0, rect.center().y), vec2(14.0, 14.0)), Icon::Puzzle, FAINT());
                let name = if same == review.commands.len() {
                    crate::i18n::trf!("Commands: {count}", count = same)
                } else {
                    crate::i18n::trf!("More: {count}", count = same)
                };
                ui.painter().text(pos2(rect.left() + 38.0, rect.center().y), Align2::LEFT_CENTER, name, FontId::proportional(13.0), FAINT());
                ui.painter().text(pos2(rect.left() + 178.0, rect.center().y), Align2::LEFT_CENTER, crate::i18n::tr("No change"), FontId::proportional(13.0), FAINT());
            }
        });
}
/// A scrolling, bounded code view with line numbers, tinted by what changed.
fn code_view(ui: &mut Ui, widgets: &mut HashMap<String, UiControl>, lines: &[Line]) {
    let height = 16.0;
    Frame::new()
        .fill(BG())
        .stroke(Stroke::new(1.0, HAIRLINE()))
        .corner_radius(8)
        .inner_margin(Margin::symmetric(0, 6))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            let scroll = ScrollArea::vertical()
                .id_salt("extension-code")
                .max_height(260.0)
                .auto_shrink([false, true])
                .show_rows(ui, height, lines.len(), |ui, range| {
                    for line in &lines[range] {
                        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
                        let (tint, ink, mark) = match line.kind {
                            LineKind::Added => (GREEN().gamma_multiply(0.1), GREEN(), "+"),
                            LineKind::Removed => (RED().gamma_multiply(0.1), RED(), "−"),
                            LineKind::Same => (Color32::TRANSPARENT, MUTED(), " "),
                            LineKind::Gap => (Color32::TRANSPARENT, FAINT(), " "),
                        };
                        ui.painter().rect_filled(rect, 0.0, tint);
                        let font = FontId::monospace(12.0);
                        let y = rect.center().y;
                        if line.kind == LineKind::Gap {
                            ui.painter().text(pos2(rect.left() + 40.0, y), Align2::LEFT_CENTER, "⋯", font, FAINT());
                            continue;
                        }
                        ui.painter().text(pos2(rect.left() + 34.0, y), Align2::RIGHT_CENTER, line.number.to_string(), font.clone(), FAINT());
                        ui.painter().text(pos2(rect.left() + 44.0, y), Align2::CENTER_CENTER, mark, font.clone(), ink);
                        let mut job = text::LayoutJob::simple_singleline(line.text.clone(), font, ink);
                        job.wrap = text::TextWrapping::truncate_at_width((rect.width() - 62.0).max(0.0));
                        let galley = ui.painter().layout_job(job);
                        ui.painter().galley(pos2(rect.left() + 54.0, y - galley.size().y / 2.0), galley, ink);
                    }
                });
            pages::region(widgets, "int-package-code-scroll", scroll.inner_rect.intersect(ui.clip_rect()), true);
        });
}
fn review_body(ui: &mut Ui, s: &mut Review, widgets: &mut HashMap<String, UiControl>, error: Option<&str>) {
    let name = s.package.name.clone();
    let compared = matches!(s.kind, Kind::Update | Kind::Again);
    pages::well(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        let api = if s.kind == Kind::Update { s.version.clone() } else { format!("{} · Lua API 1", s.version) };
        well_row(ui, crate::i18n::tr("Version"), plain(&api), |_| {});
        if s.kind == Kind::Install {
            well_row(ui, "ID", plain(&s.package.id), |_| {});
        }
        match (&s.file, s.kind) {
            (Some(file), _) => well_row(ui, crate::i18n::tr("File"), path_text(file), |_| {}),
            (None, Kind::Again) => well_row(ui, crate::i18n::tr("Changed"), plain(crate::i18n::tr("Files in the extensions folder")), |_| {}),
            (None, _) if s.copies => well_row(ui, crate::i18n::tr("Source"), plain(crate::i18n::tr("Ships with Rocket")), |_| {}),
            (None, _) => well_row(ui, crate::i18n::tr("Source"), plain(crate::i18n::tr("The extensions folder")), |_| {}),
        }
        let added = s.change.as_ref().map(|c| (c.added, c.removed));
        let code = s.code;
        well_row(
            ui,
            crate::i18n::tr("Code"),
            |ui| match added {
                Some((added, removed)) => {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    ui.label(RichText::new("main.lua").size(13.0));
                    ui.label(RichText::new(format!("+{added}")).size(13.0).color(GREEN()));
                    ui.label(RichText::new(format!("−{removed}")).size(13.0).color(RED()));
                }
                None => {
                    ui.label(RichText::new(crate::i18n::trf!("main.lua · lines: {count}", count = s.package.source.lines().count())).size(13.0));
                }
            },
            |ui| {
                let label = match (code, compared) {
                    (false, false) => crate::i18n::tr("View code"),
                    (true, false) => crate::i18n::tr("Hide code"),
                    (false, true) => crate::i18n::tr("View changes"),
                    (true, true) => crate::i18n::tr("Hide changes"),
                };
                if quiet(ui, widgets, "int-review-code", label, true) {
                    s.code = !s.code;
                }
            },
        );
    });
    if s.code {
        match &s.change {
            Some(change) => {
                code_view(ui, widgets, &change.lines);
                let n = change.changes;
                pages::note(ui, &crate::i18n::trf!("Changes: {count}", count = n));
            }
            None => {
                let lines: Vec<Line> = s.package.source.lines().enumerate().map(|(i, text)| Line {
                    kind: LineKind::Same,
                    number: i + 1,
                    text: text.to_owned(),
                }).collect();
                code_view(ui, widgets, &lines);
            }
        }
    } else {
        permissions(ui, s);
        commands(ui, s);
    }
    match s.kind {
        Kind::Install if s.package.network => pages::callout(ui, AMBER(), crate::i18n::tr("Rocket can’t confirm who made this file. It can send what you share with it to that server.")),
        Kind::Install => pages::callout(ui, AMBER(), crate::i18n::tr("Rocket can’t confirm who made this file.")),
        Kind::Enable if s.package.network => pages::callout(ui, AMBER(), &crate::i18n::trf!("{extension} can send what you share with it to that server.", extension = name)),
        _ => {}
    }
    if !s.kept.is_empty() || s.drafts > 0 {
        pages::caption(ui, crate::i18n::tr("Stays on this device"));
        pages::well(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            if !s.kept.is_empty() {
                well_row(ui, crate::i18n::tr("Connections"), plain(&crate::i18n::trf!("{connections}, including tokens and access", connections = names(&s.kept))), |_| {});
            }
            if s.drafts > 0 {
                let text = crate::i18n::trf!("Saved drafts unavailable after this change: {count}", count = s.drafts);
                let blocks = compared && s.change.as_ref().is_some_and(|c| c.added + c.removed > 0);
                let color = if blocks { AMBER() } else { TEXT() };
                well_row(ui, crate::i18n::tr("Drafts"), move |ui| {
                    ui.add(Label::new(RichText::new(text).size(13.0).color(color)).truncate());
                }, |_| {});
            }
        });
    }
    if let Some(connection) = &s.blocked {
        pages::callout(ui, RED(), &crate::i18n::trf!("{connection} has a request that may have reached the service. Check it first.", connection));
    }
    if let Some(error) = error {
        pages::callout(ui, RED(), shown(error));
    }
}

impl Rocket {
    /// The review or removal that is open, drawn above Preferences and below the file picker.
    pub(crate) fn extension_review_dialog(&mut self, ctx: &Context) {
        let Some(mut prompt) = self.integrations.prompt.take() else {
            self.integrations.prompt_frames = 0;
            return;
        };
        if self.integrations.prompt_frames < 2 {
            self.integrations.prompt_frames += 1;
            pages::raise(ctx, "extension-review");
        }
        let backend = self.backend.clone();
        let busy = self.integrations.running();
        let error = self.integrations.error.clone();
        let (image, title, danger) = match &prompt {
            Prompt::Review(r) => match r.kind {
                Kind::Enable => (Icon::Puzzle, crate::i18n::trf!("Enable {extension}", extension = r.package.name), false),
                Kind::Install => (Icon::Pull, crate::i18n::trf!("Install {extension}", extension = r.package.name), false),
                Kind::Update => (Icon::Pull, crate::i18n::trf!("Update {extension}", extension = r.package.name), false),
                Kind::Again => (Icon::Warning, crate::i18n::trf!("Review {extension} again", extension = r.package.name), false),
            },
            Prompt::Failed { .. } | Prompt::Installed { .. } => (Icon::Pull, crate::i18n::tr("Install from file").to_owned(), false),
            Prompt::Remove(r) => (Icon::Trash, r.title.clone(), true),
        };
        let mut confirm = false;
        let mut choose = false;
        let mut show = None;
        let response = pages::review(
            ctx,
            "extension-review",
            pages::ReviewHeader { image, title: &title, subtitle: "", danger },
            &mut self.frame_widgets,
            "int-package-scroll",
            true,
            |ui, part, widgets| match (part, &mut prompt) {
                (Part::Body, Prompt::Review(review)) => review_body(ui, review, widgets, error.as_deref()),
                (Part::Footer, Prompt::Review(review)) => {
                    let label = match review.kind {
                        Kind::Enable => crate::i18n::tr("Enable"),
                        Kind::Install => crate::i18n::tr("Install"),
                        Kind::Update => crate::i18n::tr("Update"),
                        Kind::Again => crate::i18n::tr("Approve"),
                    };
                    let label = if error.is_some() && !busy { crate::i18n::tr("Try again") } else { label };
                    let r = pages::primary(ui, !busy && review.blocked.is_none(), label);
                    pages::control(widgets, "int-review-approve", &r, ui.clip_rect());
                    confirm = r.clicked();
                    if busy {
                        pages::footer_status(ui, true, crate::i18n::tr("Copying the files"));
                    } else if review.kind == Kind::Again {
                        pages::footer_status(ui, false, crate::i18n::tr("It can’t run until you approve these files"));
                    } else {
                        pages::footer_status(ui, false, crate::i18n::tr("No script has run"));
                    }
                }
                (Part::Body, Prompt::Failed { file, error }) => {
                    file_row(ui, widgets, file, &mut choose);
                    let error = shown(error);
                    let detail = error.trim_end_matches("Nothing was installed.").trim_end();
                    pages::callout(ui, RED(), &format!("{detail} {}", crate::i18n::tr("Nothing was installed.")));
                }
                (Part::Footer, Prompt::Failed { .. }) => {
                    let r = pages::primary(ui, false, crate::i18n::tr("Install"));
                    pages::control(widgets, "int-review-approve", &r, ui.clip_rect());
                }
                (Part::Body, Prompt::Installed { file, name, version, .. }) => {
                    file_row(ui, widgets, file, &mut choose);
                    pages::callout(ui, GREEN(), &crate::i18n::trf!("{name} {version} is already installed.", name, version));
                }
                (Part::Footer, Prompt::Installed { id, name, .. }) => {
                    let r = pages::primary(ui, !busy, &crate::i18n::trf!("Show {name}", name));
                    pages::control(widgets, "int-review-show", &r, ui.clip_rect());
                    if r.clicked() {
                        show = Some(id.clone());
                    }
                }
                (Part::Body, Prompt::Remove(removal)) => {
                    pages::well(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        for (label, value) in &removal.rows {
                            well_row(ui, label, plain(value), |_| {});
                        }
                    });
                    if let Some(connection) = &removal.blocked {
                        pages::callout(ui, RED(), &crate::i18n::trf!("{connection} has a request that may have reached the service. Check it first.", connection));
                    }
                    if let Some(error) = &error {
                        pages::callout(ui, RED(), shown(error));
                    }
                }
                (Part::Footer, Prompt::Remove(removal)) => {
                    let r = pages::danger(ui, !busy && removal.blocked.is_none(), crate::i18n::tr("Remove"));
                    pages::control(widgets, "int-review-approve", &r, ui.clip_rect());
                    confirm = r.clicked();
                    if busy {
                        pages::footer_status(ui, true, crate::i18n::tr("Removing"));
                    }
                }
            },
        );
        let closing = response.should_close() && !busy;
        let s = &mut self.integrations;
        if !closing {
            s.prompt = Some(prompt);
        } else {
            s.error = None;
        }
        if let Some(backend) = backend {
            if confirm && let Some(Prompt::Remove(_)) = &s.prompt {
                s.confirm_removal(backend);
            } else if confirm {
                s.confirm(backend);
            } else if let Some(id) = show {
                s.prompt = None;
                s.details = true;
                s.perform(backend, Action::Package(id));
            }
        }
        if choose {
            self.browse(PickTarget::ExtensionFile, "");
        }
    }
}

/// The chosen file, with Choose file to pick another.
fn file_row(ui: &mut Ui, widgets: &mut HashMap<String, UiControl>, file: &std::path::Path, choose: &mut bool) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let (rect, _) = ui.allocate_exact_size(vec2(56.0, CONTROL), Sense::hover());
        ui.painter().text(rect.left_center(), Align2::LEFT_CENTER, crate::i18n::tr("File"), FontId::proportional(13.0), MUTED());
        let width = (ui.available_width() - 104.0).max(80.0);
        let mut text = crate::views::display_path(file);
        ui.add_enabled_ui(false, |ui| {
            ui.add_sized([width, CONTROL], TextEdit::singleline(&mut text).vertical_align(Align::Center));
        });
        let r = ui.add_sized([96.0, CONTROL], Button::new(crate::i18n::tr("Choose file")));
        pages::control(widgets, "int-review-choose", &r, ui.clip_rect());
        *choose = r.clicked();
    });
}
