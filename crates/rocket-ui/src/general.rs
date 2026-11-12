//! The settings on Preferences → General beyond the window and updates: where new
//! repositories go, how Rocket fetches, switches and names branches, and how diffs and
//! the commit list read. Every one starts at Rocket's behaviour without it.
use crate::{
    app::{Rocket, UiControl},
    backend::{Outcome, user_error},
    i18n::{Language, tr},
    pages,
    preferences::{bar_switch_row, card_frame, card_header, choices, divider, status_line, text_row},
    repository_forms::Status,
    style::*,
};
use eframe::egui::*;
use rocket_git::{CancellationToken, FetchTags, RepoPath, Repository, SubmoduleEntry};
use std::{
    collections::{BTreeMap, HashMap},
    path::{Path, PathBuf},
};

/// The label column of the page's field rows.
const LABEL: f32 = 172.0;
/// Diff font sizes offered; 13 is the size diffs always had.
pub const DIFF_FONTS: [u8; 6] = [11, 12, 13, 14, 15, 16];
const TAGS: [(FetchTags, &str, &str); 3] = [
    (FetchTags::None, "none", "No tags"),
    (FetchTags::RemoteDefault, "default", "Usual tags"),
    (FetchTags::All, "all", "All tags"),
];
const SPACES: [(BranchSpace, &str, &str); 3] = [
    (BranchSpace::Keep, "none", "Not allowed"),
    (BranchSpace::Hyphen, "hyphen", "-"),
    (BranchSpace::Underscore, "underscore", "_"),
];

/// What a space typed into a branch name becomes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BranchSpace {
    /// It stays, and the name check asks for another name.
    #[default]
    Keep,
    Hyphen,
    Underscore,
}
impl BranchSpace {
    fn character(self) -> Option<char> {
        match self {
            Self::Keep => None,
            Self::Hyphen => Some('-'),
            Self::Underscore => Some('_'),
        }
    }
    /// Changes the spaces in a name as it is typed or pasted. Leading ones go, since a
    /// name can't start with a dash.
    pub fn apply(self, name: &mut String) {
        if let Some(replacement) = self.character()
            && name.chars().any(char::is_whitespace)
        {
            *name = name
                .trim_start()
                .chars()
                .map(|c| if c.is_whitespace() { replacement } else { c })
                .collect();
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    /// Where Clone and New repository start; `None` keeps their usual start.
    pub source_folder: Option<PathBuf>,
    /// Where the Fetch dialog starts, and what automatic fetch includes.
    pub fetch_tags: FetchTags,
    pub branch_space: BranchSpace,
    /// A branch switch also updates initialized, clean submodules.
    pub submodules_on_checkout: bool,
    /// Commit & Push is the main button in Changes and takes Ctrl+Enter.
    pub push_after_commit: bool,
    pub diff_marks: bool,
    pub diff_font: u8,
    pub diff_syntax: bool,
    pub compact_labels: bool,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            source_folder: None,
            fetch_tags: FetchTags::None,
            branch_space: BranchSpace::Keep,
            submodules_on_checkout: false,
            push_after_commit: false,
            diff_marks: true,
            diff_font: 13,
            diff_syntax: true,
            compact_labels: false,
        }
    }
}
impl Settings {
    pub(crate) fn valid(&self) -> bool {
        DIFF_FONTS.contains(&self.diff_font)
            && self.source_folder.as_ref().is_none_or(|p| p.is_absolute())
    }
    /// Makes the settings the diff and commit list read available to every window.
    pub(crate) fn publish(&self, ctx: &Context) {
        let options = Options {
            marks: self.diff_marks,
            font: self.diff_font,
            syntax: self.diff_syntax,
            compact_labels: self.compact_labels,
            push_after_commit: self.push_after_commit,
        };
        ctx.data_mut(|d| d.insert_temp(options_id(), options));
    }
}

/// What views read while they draw. It lives on the egui context, so the main window and
/// the review windows follow one value without a process-wide global.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Options {
    pub marks: bool,
    pub font: u8,
    pub syntax: bool,
    pub compact_labels: bool,
    pub push_after_commit: bool,
}
impl Default for Options {
    fn default() -> Self {
        let settings = Settings::default();
        Self {
            marks: settings.diff_marks,
            font: settings.diff_font,
            syntax: settings.diff_syntax,
            compact_labels: settings.compact_labels,
            push_after_commit: settings.push_after_commit,
        }
    }
}
fn options_id() -> Id {
    Id::unique("rocket-general-options")
}
pub(crate) fn options(ctx: &Context) -> Options {
    ctx.data(|d| d.get_temp(options_id())).unwrap_or_default()
}

/// A diff row's height: today's 22 px, growing with the larger fonts.
pub(crate) fn diff_row_height(font: u8) -> f32 {
    (f32::from(font) + 9.0).max(22.0)
}

/// Two paths name one folder: case, a trailing separator and an extended-length prefix
/// don't count on Windows.
pub(crate) fn same_folder(a: &Path, b: &Path) -> bool {
    let text = |p: &Path| crate::views::display_path(p).trim_end_matches(['\\', '/']).to_lowercase();
    text(a) == text(b)
}

/// The Source code folder check, in the terms of the form's: New is a usable folder,
/// Exists a file and MissingParent a path that isn't there.
fn check_folder(path: &Path) -> Status {
    match std::fs::metadata(path) {
        Ok(found) if found.is_dir() => Status::New,
        Ok(_) => Status::Exists,
        Err(_) => Status::MissingParent,
    }
}

/// A row with its label in the page's column.
fn field(ui: &mut Ui, label: &str, content: impl FnOnce(&mut Ui)) {
    ui.add_space(4.0);
    text_row(ui, label, LABEL, content);
    ui.add_space(4.0);
}

fn language_label(language: Language) -> String {
    match language {
        language if language.is_preview() => format!("{} ({})", language.name(), tr("Preview")),
        language => language.name().to_owned(),
    }
}

/// Show the detected language until the user picks one. All options use their own
/// names, with unfinished translations marked in the list and selected value.
/// MCP: `pref-language` opens the list; `pref-language-<tag>` (lower case) picks.
pub(crate) fn language_row(ui: &mut Ui, widgets: &mut HashMap<String, UiControl>, choice: &mut Option<Language>) {
    field(ui, tr("Languages"), |ui| {
        let selected = crate::i18n::resolve(*choice);
        let dialog_layer = ui.layer_id();
        let combo = ComboBox::from_id_salt("pref-language")
            .selected_text(language_label(selected))
            .width(ui.available_width().min(300.0))
            .height(360.0)
            .show_ui(ui, |ui| {
                // Preferences is a modal. Keep the popup above it for real pointer
                // input as well as synthetic input, as the Shell selector does.
                ui.ctx().set_sublayer(dialog_layer, ui.layer_id());
                ui.spacing_mut().item_spacing.y = 0.0;
                for &language in crate::i18n::offered() {
                    let row = ui.selectable_label(selected == language, language_label(language));
                    widgets.insert(format!("pref-language-{}", language.tag().to_ascii_lowercase()), (&row).into());
                    if row.clicked() {
                        // Even picking the detected language is an explicit override.
                        *choice = Some(language);
                        ui.close();
                    }
                }
            });
        widgets.insert("pref-language".into(), (&combo.response).into());
    });
    Frame::new().inner_margin(Margin::symmetric(14, 6)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.add(Label::new(RichText::new(tr("Preview translations are incomplete. Untranslated text uses English.")).size(12.0).color(HINT())).wrap());
    });
}

/// One segmented row: `picked` is the option clicked.
fn segmented<T: Copy + PartialEq>(
    ui: &mut Ui,
    widgets: &mut HashMap<String, UiControl>,
    prefix: &str,
    value: &mut T,
    options: &[(T, &str, &str)],
) {
    let items: Vec<(&str, bool)> = options.iter().map(|(v, _, label)| (*label, *v == *value)).collect();
    let ids: Vec<String> = options.iter().map(|(_, id, _)| format!("{prefix}-{id}")).collect();
    if let Some(i) = choices(ui, widgets, &items, &ids) {
        *value = options[i].0;
    }
}

impl Rocket {
    /// Shows the language Preferences picked, or the system's. A change applies from the
    /// next frame in every window; messages already shown keep the language they had.
    pub(crate) fn update_language(&mut self, ctx: &Context) {
        let wanted = crate::i18n::resolve(self.language);
        if wanted != crate::i18n::current() {
            crate::i18n::set(wanted);
            // Han characters take the new language's forms.
            ctx.set_fonts(crate::fonts::definitions());
            ctx.request_repaint();
        }
    }

    /// The page opened: its field shows the saved folder, checked afresh.
    pub(crate) fn start_general_page(&mut self) {
        self.preferences.source_text = self
            .general
            .source_folder
            .as_ref()
            .map(|p| crate::views::display_path(p))
            .unwrap_or_default();
        self.preferences.source_check = Default::default();
    }

    /// Browse chose a folder.
    pub(crate) fn source_folder_picked(&mut self, path: &Path) {
        self.general.source_folder = Some(path.to_path_buf());
        self.start_general_page();
    }

    /// The saved folder Clone and New repository may start in. The MCP test host keeps
    /// its windows inside the test folder.
    pub(crate) fn source_folder(&self) -> Option<PathBuf> {
        self.general.source_folder.clone().filter(|path| {
            self.browse_root
                .as_deref()
                .is_none_or(|root| crate::folder_browser::inside(path, root))
        })
    }

    /// Repositories, Diff view and Commit list. Returns whether Browse was clicked.
    pub(crate) fn general_cards(&mut self, ui: &mut Ui) -> bool {
        let mut browse = false;
        card_frame(ui, |ui| {
            card_header(ui, tr("Repositories"), |_| {});
            browse = self.source_row(ui);
            divider(ui);
            let widgets = &mut self.frame_widgets;
            let general = &mut self.general;
            field(ui, tr("Tags in fetch"), |ui| {
                segmented(ui, widgets, "pref-fetch-tags", &mut general.fetch_tags, &TAGS);
            });
            divider(ui);
            field(ui, tr("Space in branch names"), |ui| {
                ui.spacing_mut().item_spacing.x = 14.0;
                segmented(ui, widgets, "pref-branch-space", &mut general.branch_space, &SPACES);
                let mut example = "fix login".to_owned();
                general.branch_space.apply(&mut example);
                if general.branch_space != BranchSpace::Keep {
                    ui.label(RichText::new(format!("fix login → {example}")).monospace().size(12.0).color(HINT()));
                }
            });
            divider(ui);
            bar_switch_row(ui, widgets, "pref-submodules-on-checkout", &mut general.submodules_on_checkout, tr("Update submodules on checkout"), "", "");
            divider(ui);
            bar_switch_row(ui, widgets, "pref-push-after-commit", &mut general.push_after_commit, tr("Push after commit"), "", "");
        });
        card_frame(ui, |ui| {
            card_header(ui, tr("Diff view"), |_| {});
            let general = &mut self.general;
            Frame::new().inner_margin(Margin::symmetric(14, 12)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                self.preferences
                    .diff_preview
                    .get_or_insert_with(Default::default)
                    .show(ui, general.diff_marks, general.diff_font, general.diff_syntax);
            });
            divider(ui);
            let widgets = &mut self.frame_widgets;
            field(ui, tr("Font size"), |ui| {
                let labels = DIFF_FONTS.map(|size| size.to_string());
                let ids = DIFF_FONTS.map(|size| size.to_string());
                let options: Vec<(u8, &str, &str)> = DIFF_FONTS
                    .iter()
                    .zip(ids.iter().zip(&labels))
                    .map(|(size, (id, label))| (*size, id.as_str(), label.as_str()))
                    .collect();
                segmented(ui, widgets, "pref-diff-font", &mut general.diff_font, &options);
            });
            divider(ui);
            bar_switch_row(ui, widgets, "pref-diff-marks", &mut general.diff_marks, tr("Show − and + marks"), "", "");
            divider(ui);
            bar_switch_row(ui, widgets, "pref-diff-syntax", &mut general.diff_syntax, tr("Syntax highlighting"), "", "");
        });
        card_frame(ui, |ui| {
            card_header(ui, tr("Commit list"), |_| {});
            let general = &mut self.general;
            Frame::new().inner_margin(Margin::symmetric(14, 12)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                crate::views::label_preview(ui, general.compact_labels);
            });
            divider(ui);
            let widgets = &mut self.frame_widgets;
            bar_switch_row(ui, widgets, "pref-compact-labels", &mut general.compact_labels, tr("Compact branch labels"), "", "");
        });
        browse
    }

    /// The folder field. Text that is not yet a full path waits; the setting keeps its
    /// value until it is one.
    fn source_row(&mut self, ui: &mut Ui) -> bool {
        const CLEAR: f32 = 20.0;
        let mut browse = false;
        let mut clear = false;
        let widgets = &mut self.frame_widgets;
        let text = &mut self.preferences.source_text;
        let check = &mut self.preferences.source_check;
        let general = &mut self.general;
        let mut problem: Option<String> = None;
        field(ui, tr("Source code folder"), |ui| {
            let width = (ui.available_width() - 86.0 - ui.spacing().item_spacing.x).max(80.0);
            let set = !text.is_empty();
            let r = ui.add_sized(
                [width, CONTROL],
                TextEdit::singleline(text)
                    .hint_text(RichText::new(tr("Next to the latest repository")).font(FontId::proportional(13.0)).color(FAINT()))
                    .font(FontId::monospace(12.0))
                    .align(Align2::LEFT_CENTER)
                    .margin(Margin { left: 8, right: if set { 52 } else { 8 }, top: 2, bottom: 2 }),
            );
            pages::control(widgets, "pref-source-folder", &r, ui.clip_rect());
            if r.changed() {
                let typed = text.trim();
                if typed.is_empty() {
                    general.source_folder = None;
                } else if Path::new(typed).is_absolute() {
                    general.source_folder = Some(PathBuf::from(typed));
                }
            }
            let typed = text.trim();
            let full = Path::new(typed).is_absolute();
            let status = check.status(ui.ctx(), full.then(|| PathBuf::from(typed)), None, check_folder);
            problem = if typed.is_empty() {
                None
            } else if !full {
                Some(tr("Enter a full path, such as C:\\Code.").into())
            } else {
                match status {
                    Some(Status::MissingParent) => Some(tr("Folder not found. Clone starts next to the latest repository.").into()),
                    Some(Status::Exists) => Some(tr("This is a file. Clone starts next to the latest repository.").into()),
                    Some(Status::Refused(reason)) => Some(reason),
                    _ => None,
                }
            };
            if problem.is_some() {
                ui.painter().rect_stroke(r.rect, 6.0, Stroke::new(1.0, WARNING().gamma_multiply(0.6)), StrokeKind::Inside);
                let mark = Rect::from_center_size(pos2(r.rect.right() - 36.0, r.rect.center().y), vec2(14.0, 14.0));
                icon(ui.painter(), mark, Icon::Warning, WARNING());
            }
            if set {
                let spot = Rect::from_center_size(pos2(r.rect.right() - 14.0, r.rect.center().y), vec2(CLEAR, CLEAR));
                let mut over = ui.new_child(UiBuilder::new().max_rect(spot));
                let close = crate::chrome::icon_button(&mut over, Icon::Close, true, false, CLEAR)
                    .on_hover_text(tr("Clear the folder"));
                pages::control(widgets, "pref-source-clear", &close, ui.clip_rect());
                clear = close.clicked();
            }
            let r = ui.add_sized([86.0, CONTROL], Button::new(tr("Browse")));
            pages::control(widgets, "pref-source-browse", &r, ui.clip_rect());
            browse = r.clicked();
        });
        if let Some(problem) = problem {
            ui.horizontal(|ui| {
                ui.add_space(14.0 + LABEL + 12.0);
                status_line(ui, WARNING(), &problem);
            });
            ui.add_space(8.0);
        }
        if clear {
            self.general.source_folder = None;
            self.start_general_page();
        }
        browse
    }

    /// A branch switch finished: its tab updates submodules once its refresh has ended.
    pub(crate) fn poll_submodule_update(&mut self) {
        if self.write.is_some() || self.quitting {
            return;
        }
        let Some(tab) = self.tabs.iter_mut().find(|t| t.submodule_update && !t.loading()) else {
            return;
        };
        tab.submodule_update = false;
        let id = tab.id;
        let root = self.submodule_fixture_root.clone();
        self.execute(id, crate::backend::Action::UpdateSubmodules(root));
    }
}

/// Why a submodule stays where it is after a switch; `None` when it can be updated.
fn left_alone(entry: &SubmoduleEntry, all: &[SubmoduleEntry]) -> Option<&'static str> {
    let below = || all.iter().filter(|e| e.directory.starts_with(&entry.directory) && e.directory != entry.directory);
    if !entry.initialized {
        Some("not initialized")
    } else if entry.blocked.is_some() || below().any(|e| e.blocked.is_some()) {
        Some("needs attention")
    } else if entry.dirty || below().any(|e| e.dirty) {
        Some("local changes")
    } else {
        None
    }
}

/// The result line: what moved and what stayed, naming a lone exception.
fn update_summary(updated: usize, left: &BTreeMap<String, &'static str>) -> String {
    let noted: Vec<_> = left.iter().filter(|(_, why)| **why != "not initialized").collect();
    let moved = match updated {
        0 => None,
        1 => Some("1 submodule updated".to_owned()),
        n => Some(format!("{n} submodules updated")),
    };
    match (moved, noted.as_slice()) {
        (Some(moved), []) => moved,
        (None, []) => "Submodules are up to date".to_owned(),
        (moved, [(path, why)]) => match moved {
            Some(moved) => format!("{moved} · {path} left alone, {why}"),
            None => format!("{path} left alone, {why}"),
        },
        (moved, many) => match moved {
            Some(moved) => format!("{moved} · {} left alone", many.len()),
            None => format!("{} submodules left alone", many.len()),
        },
    }
}

/// After a branch switch: moves initialized, clean submodules to the commits the branch
/// records, parents before their nested children. The switch in Preferences is the
/// go-ahead, so no review opens; nothing is forced. Uninitialized ones stay out, so a
/// branch can't make Rocket clone a URL it brought along. `root` is the MCP fixture folder.
pub(crate) fn update_submodules(
    repo: &Repository,
    root: Option<&Path>,
    cancel: &CancellationToken,
) -> rocket_git::Result<Outcome> {
    let mut out = Outcome { command_succeeded: true, ..Default::default() };
    let mut updated = 0;
    let mut left = BTreeMap::new();
    let mut log = Vec::new();
    let mut stopped = None;
    'depths: for depth in 0..=16 {
        let all = repo.submodules(cancel)?;
        if !all.iter().any(|e| e.depth >= depth) {
            break;
        }
        let mut ready: Vec<RepoPath> = Vec::new();
        for entry in all.iter().filter(|e| e.depth == depth) {
            let under_held = left.keys().any(|held: &String| {
                entry.path.display_lossy().starts_with(&format!("{held}/"))
            });
            if under_held {
                continue;
            }
            let behind = entry.head.as_ref() != Some(&entry.target);
            match left_alone(entry, &all) {
                Some(why) if behind => {
                    left.insert(entry.path.display_lossy().into_owned(), why);
                }
                None if behind => ready.push(entry.path.clone()),
                _ => {}
            }
        }
        for paths in ready.chunks(100) {
            let preview = repo.preview_submodule_batch(paths, root, cancel)?;
            let report = repo.apply_submodule_batch(&preview, cancel)?;
            for (path, result) in report.results {
                let path = path.display_lossy().into_owned();
                match result.and_then(|r| {
                    r.result.and(r.verification)?;
                    Ok(r.recovery_reference)
                }) {
                    Ok(recovery) => {
                        updated += 1;
                        log.push(match recovery {
                            Some(reference) => format!("{path}: updated; previous commit kept at {reference}"),
                            None => format!("{path}: updated"),
                        });
                    }
                    Err(error) => {
                        stopped = Some(format!("{path}: {}", user_error(&error)));
                        break 'depths;
                    }
                }
            }
            if !report.skipped.is_empty() {
                stopped = Some(format!("{} updates were not started.", report.skipped.len()));
                break 'depths;
            }
        }
    }
    for (path, why) in &left {
        if *why == "not initialized" {
            log.push(format!("{path}: not initialized"));
        } else {
            out.details.push(format!("{path}: left alone, {why}"));
        }
    }
    out.raw_output = log.join("\n");
    if let Some(reason) = stopped {
        out.command_succeeded = false;
        out.recovery_pending = true;
        out.summary = "Submodule update stopped. Inspect the checkout before retrying.".into();
        out.details.push(reason);
    } else {
        // Nothing moved and nothing needs a look: a repository with no work here shows nothing.
        out.quiet = updated == 0 && out.details.is_empty();
        out.summary = update_summary(updated, &left);
    }
    Ok(out)
}

#[cfg(all(test, windows))]
mod flow_tests {
    use super::*;
    use crate::{
        app::{DialogKind, Intent, View},
        batch::flow_tests::{click, frame, open, wait},
        test_common::{self as common, Fixture},
    };
    use std::{
        thread,
        time::{Duration, Instant},
    };

    const CASE: &str = "ROCKET_GENERAL_UI_CASE";

    fn committed() -> Fixture {
        let f = Fixture::new();
        f.write("a.txt", "1\n");
        f.commit("base");
        f
    }
    /// Frames until `done` holds.
    fn until(ctx: &Context, app: &mut Rocket, done: impl Fn(&Rocket) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while !done(app) {
            app.poll(ctx);
            frame(ctx, app, vec![]);
            assert!(Instant::now() < deadline, "the form did not settle");
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn clone_starts_in_the_source_folder_until_it_goes_missing() {
        common::isolated(CASE, "general::flow_tests::clone_starts_in_the_source_folder_until_it_goes_missing", || {
            let f = committed();
            let ctx = Context::default();
            let mut app = open(&ctx, &f);
            let parent = |app: &Rocket| app.dialog.as_ref().unwrap().create.as_ref().unwrap().parent.clone();
            app.show_dialog(DialogKind::Clone);
            frame(&ctx, &mut app, vec![]);
            let usual = parent(&app);

            let source = f.root.canonicalize().unwrap().join("code");
            std::fs::create_dir(&source).unwrap();
            let shown = crate::views::display_path(&source);
            assert_ne!(usual, shown);
            app.dialog = None;
            app.general.source_folder = Some(source.clone());
            app.show_dialog(DialogKind::Clone);
            until(&ctx, &mut app, |app| app.dialog.as_ref().unwrap().create.as_ref().is_some_and(|f| !f.start_pending()));
            assert_eq!(parent(&app), shown);

            app.dialog = None;
            app.general.source_folder = Some(source.join("gone"));
            app.show_dialog(DialogKind::Clone);
            frame(&ctx, &mut app, vec![]);
            assert_eq!(parent(&app), format!("{shown}\\gone"), "shown at once, without waiting for the disk");
            until(&ctx, &mut app, |app| app.dialog.as_ref().unwrap().create.as_ref().is_some_and(|f| !f.start_pending()));
            assert_eq!(parent(&app), usual);

            // A folder typed meanwhile stays.
            app.dialog = None;
            app.show_dialog(DialogKind::Clone);
            frame(&ctx, &mut app, vec![]);
            app.dialog.as_mut().unwrap().create.as_mut().unwrap().parent = shown.clone();
            until(&ctx, &mut app, |app| app.dialog.as_ref().unwrap().create.as_ref().is_some_and(|f| !f.start_pending()));
            assert_eq!(parent(&app), shown);
        });
    }

    #[test]
    fn new_repository_needs_a_name_inside_the_source_folder() {
        common::isolated(CASE, "general::flow_tests::new_repository_needs_a_name_inside_the_source_folder", || {
            let f = committed();
            let ctx = Context::default();
            let mut app = open(&ctx, &f);
            let source = f.root.canonicalize().unwrap().join("code");
            std::fs::create_dir(&source).unwrap();
            app.general.source_folder = Some(source.clone());
            app.show_dialog(DialogKind::NewRepository);
            let settled = |app: &Rocket| app.dialog.as_ref().unwrap().create.as_ref().is_some_and(|f| !f.start_pending() && !f.checking());
            until(&ctx, &mut app, settled);
            let start = format!("{}\\", crate::views::display_path(&source));
            assert_eq!(app.dialog.as_ref().unwrap().first, start);
            frame(&ctx, &mut app, vec![]);
            assert!(!app.frame_widgets["dialog-submit"].enabled, "the source folder itself isn't offered");
            app.dialog.as_mut().unwrap().first = format!("{start}fresh");
            frame(&ctx, &mut app, vec![]);
            until(&ctx, &mut app, settled);
            frame(&ctx, &mut app, vec![]);
            assert!(app.frame_widgets["dialog-submit"].enabled);
        });
    }

    #[test]
    fn dialogs_start_from_the_chosen_tags_and_spaces() {
        common::isolated(CASE, "general::flow_tests::dialogs_start_from_the_chosen_tags_and_spaces", || {
            let f = committed();
            let ctx = Context::default();
            let mut app = open(&ctx, &f);
            app.show_dialog(DialogKind::Fetch);
            assert_eq!(app.dialog.as_ref().unwrap().fetch_tags, FetchTags::None, "today's start");
            app.dialog = None;
            app.general.fetch_tags = FetchTags::All;
            app.show_dialog(DialogKind::Fetch);
            assert_eq!(app.dialog.as_ref().unwrap().fetch_tags, FetchTags::All);
            app.dialog = None;

            app.general.branch_space = BranchSpace::Hyphen;
            app.dispatch(Intent::Dialog(DialogKind::NewBranch));
            frame(&ctx, &mut app, vec![]);
            click(&ctx, &mut app, "dialog-first");
            frame(&ctx, &mut app, vec![Event::Text("fix login page".into())]);
            assert_eq!(app.dialog.as_ref().unwrap().first, "fix-login-page");
        });
    }

    #[test]
    fn push_after_commit_swaps_the_buttons_only_where_a_push_can_start() {
        common::isolated(CASE, "general::flow_tests::push_after_commit_swaps_the_buttons_only_where_a_push_can_start", || {
            let f = committed();
            f.write("b.txt", "2\n");
            f.run(&["add", "b.txt"]);
            let ctx = Context::default();
            let mut app = open(&ctx, &f);
            app.general.push_after_commit = true;
            let tab = app.tabs[0].id;
            app.dispatch(Intent::View(View::Changes));
            app.tabs[0].draft = "add b".into();
            wait(&ctx, &mut app);
            app.poll(&ctx);
            frame(&ctx, &mut app, vec![]);
            let width = |app: &Rocket, id: &str| app.frame_widgets[id].rect.width();
            assert!(width(&app, "commit") > width(&app, "commit-push"), "no remote: Commit stays the main button");

            f.run(&["remote", "add", "origin", f.root.join("origin.git").to_str().unwrap()]);
            app.dispatch(Intent::Refresh);
            wait(&ctx, &mut app);
            frame(&ctx, &mut app, vec![]);
            assert!(width(&app, "commit-push") > width(&app, "commit"), "Commit & Push takes the main button");
            let enter = Event::Key {
                key: Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Modifiers::CTRL,
            };
            let mut intents = Vec::new();
            let mut output = ctx.run_ui(RawInput { events: vec![enter], ..Default::default() }, |ui| {
                app.run_shortcuts(ui.ctx(), &mut intents);
            });
            output.textures_delta.clear();
            assert!(matches!(intents.as_slice(), [Intent::CommitPush]), "Ctrl+Enter goes to the review");
            click(&ctx, &mut app, "commit-push");
            assert!(matches!(app.dialog.as_ref().map(|d| &d.kind), Some(DialogKind::Workflows)), "the review opens first");
            assert_eq!(app.tabs.iter().find(|t| t.id == tab).unwrap().draft, "add b", "nothing was committed");
            assert_eq!(f.run(&["rev-list", "--count", "HEAD"]), b"1\n");
        });
    }

    /// A parent whose main and topic branches record different commits of one child.
    fn parent_with_child() -> (Fixture, Fixture, String, String) {
        let source = committed();
        let old = String::from_utf8(source.run(&["rev-parse", "HEAD"])).unwrap().trim().to_owned();
        source.write("a.txt", "2\n");
        source.commit("advance");
        let new = String::from_utf8(source.run(&["rev-parse", "HEAD"])).unwrap().trim().to_owned();
        let f = committed();
        f.run(&["-c", "protocol.file.allow=always", "submodule", "add", source.repository.to_str().unwrap(), "vendor/child"]);
        f.commit("add child");
        f.run(&["branch", "topic"]);
        f.run(&["-C", "vendor/child", "checkout", "--quiet", "--detach", &old]);
        f.run(&["update-index", "--cacheinfo", &format!("160000,{old},vendor/child")]);
        f.run(&["commit", "--quiet", "-m", "older child"]);
        (f, source, old, new)
    }
    fn child_head(f: &Fixture) -> String {
        String::from_utf8(f.run(&["-C", "vendor/child", "rev-parse", "HEAD"])).unwrap().trim().to_owned()
    }

    #[test]
    fn switching_branch_updates_a_clean_submodule_when_the_setting_is_on() {
        common::isolated(CASE, "general::flow_tests::switching_branch_updates_a_clean_submodule_when_the_setting_is_on", || {
            let (f, _source, old, new) = parent_with_child();
            let ctx = Context::default();
            let mut app = open(&ctx, &f);
            let tab = app.tabs[0].id;
            let switch = |app: &mut Rocket, name: &str| {
                app.dispatch(Intent::Execute(tab, crate::backend::Action::SwitchBranch(name.as_bytes().to_vec())));
                wait(&ctx, app);
            };
            // Off, as today: the child stays on its old commit.
            switch(&mut app, "topic");
            assert_eq!(child_head(&f), old);
            switch(&mut app, "main");

            app.general.submodules_on_checkout = true;
            let before = app.activities.len();
            switch(&mut app, "topic");
            assert_eq!(child_head(&f), new, "the clean child follows the branch");
            let last = app.activities.last().unwrap();
            assert_eq!(last.title, "submodule-checkout");
            assert!(last.outcome.command_succeeded);
            assert_eq!(last.outcome.summary, "1 submodule updated");
            assert_eq!(app.activities.len(), before + 2, "the switch and the update each get a line");

            // Local changes in the child stop nothing and lose nothing.
            f.write("vendor/child/a.txt", "edited\n");
            switch(&mut app, "main");
            assert_eq!(child_head(&f), new, "a changed child stays where it is");
            let last = app.activities.last().unwrap();
            assert!(last.outcome.command_succeeded);
            assert_eq!(last.outcome.summary, "vendor/child left alone, local changes");
            assert_eq!(std::fs::read_to_string(f.repository.join("vendor/child/a.txt")).unwrap(), "edited\n");

            // The session's review option opens its page once the update is done.
            f.run(&["-C", "vendor/child", "checkout", "--", "a.txt"]);
            app.review_submodules_after_switch = true;
            switch(&mut app, "topic");
            assert!(matches!(app.dialog.as_ref().map(|d| &d.kind), Some(DialogKind::Submodules)));
        });
    }

    #[test]
    fn a_repository_with_nothing_to_update_shows_nothing_after_a_switch() {
        common::isolated(CASE, "general::flow_tests::a_repository_with_nothing_to_update_shows_nothing_after_a_switch", || {
            let f = committed();
            f.run(&["branch", "topic"]);
            let ctx = Context::default();
            let mut app = open(&ctx, &f);
            app.general.submodules_on_checkout = true;
            let tab = app.tabs[0].id;
            app.dispatch(Intent::Execute(tab, crate::backend::Action::SwitchBranch(b"topic".to_vec())));
            wait(&ctx, &mut app);
            assert_eq!(app.activities.len(), 1, "only the switch itself");
            assert_eq!(app.activities[0].title, "branch-switch");
            assert!(!app.tabs[0].submodule_update);
        });
    }

    #[test]
    fn a_path_on_the_page_is_saved_once_it_is_full_and_warned_about_when_unusable() {
        common::isolated(CASE, "general::flow_tests::a_path_on_the_page_is_saved_once_it_is_full_and_warned_about_when_unusable", || {
            let f = committed();
            let ctx = Context::default();
            let mut app = open(&ctx, &f);
            app.show_preferences(crate::preferences::Page::General);
            frame(&ctx, &mut app, vec![]);
            frame(&ctx, &mut app, vec![]);
            assert!(app.frame_widgets.contains_key("pref-source-folder"), "shown at the default scroll");
            assert!(app.frame_widgets.contains_key("pref-source-browse"));
            assert!(!app.frame_widgets.contains_key("pref-source-clear"), "nothing to clear yet");
            click(&ctx, &mut app, "pref-source-folder");
            frame(&ctx, &mut app, vec![Event::Text("Code".into())]);
            assert_eq!(app.preferences.source_text, "Code");
            assert_eq!(app.general.source_folder, None, "a partial path isn't saved");

            app.preferences.source_text.clear();
            let missing = f.root.canonicalize().unwrap().join("gone");
            frame(&ctx, &mut app, vec![Event::Text(missing.display().to_string())]);
            assert_eq!(app.general.source_folder, Some(missing.clone()));
            frame(&ctx, &mut app, vec![]);
            assert!(app.frame_widgets.contains_key("pref-source-clear"));
            click(&ctx, &mut app, "pref-source-clear");
            assert_eq!(app.general.source_folder, None);
            assert_eq!(app.preferences.source_text, "");
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_labels_keep_native_names_and_preview_status() {
        assert_eq!(language_label(Language::English), "English");
        assert_eq!(language_label(Language::GermanSwiss), "Deutsch (Schweiz) (Preview)");
        crate::i18n::scoped(Language::ChineseSimplified, || {
            assert_eq!(language_label(Language::Japanese), "日本語 (预览)");
            assert_eq!(language_label(Language::English), "English");
        });
    }

    #[test]
    fn spaces_become_the_chosen_character_as_a_name_is_typed() {
        let typed = |space: BranchSpace, text: &str| {
            let mut name = text.to_owned();
            space.apply(&mut name);
            name
        };
        assert_eq!(typed(BranchSpace::Keep, "fix login"), "fix login");
        assert_eq!(typed(BranchSpace::Hyphen, "fix login page"), "fix-login-page");
        assert_eq!(typed(BranchSpace::Underscore, "fix login"), "fix_login");
        // A dash can't start a name, and a space being typed last must stay usable.
        assert_eq!(typed(BranchSpace::Hyphen, "  fix"), "fix");
        assert_eq!(typed(BranchSpace::Hyphen, "fix "), "fix-");
        assert_eq!(typed(BranchSpace::Hyphen, "fix\u{a0}文"), "fix-文");
    }

    #[test]
    fn folders_match_without_case_or_a_trailing_separator() {
        assert!(same_folder(Path::new("C:\\Code\\"), Path::new("c:\\code")));
        assert!(!same_folder(Path::new("C:\\Code\\app"), Path::new("C:\\Code")));
        assert!(same_folder(Path::new(r"\\?\C:\Code"), Path::new("C:\\Code\\")));
    }

    #[test]
    fn the_result_line_names_a_lone_exception() {
        let left = |entries: &[(&str, &'static str)]| {
            entries.iter().map(|(p, w)| ((*p).to_owned(), *w)).collect::<BTreeMap<_, _>>()
        };
        assert_eq!(update_summary(2, &left(&[])), "2 submodules updated");
        assert_eq!(update_summary(1, &left(&[])), "1 submodule updated");
        assert_eq!(
            update_summary(2, &left(&[("vendor/ui", "local changes")])),
            "2 submodules updated · vendor/ui left alone, local changes"
        );
        assert_eq!(
            update_summary(0, &left(&[("a", "local changes"), ("b", "needs attention")])),
            "2 submodules left alone"
        );
        // Not initializing a submodule is a choice, not something to report.
        assert_eq!(update_summary(1, &left(&[("docs", "not initialized")])), "1 submodule updated");
    }

    #[test]
    fn options_follow_the_settings_on_their_context() {
        let ctx = Context::default();
        assert_eq!(options(&ctx), Options::default());
        let settings = Settings { diff_font: 16, diff_marks: false, compact_labels: true, ..Default::default() };
        settings.publish(&ctx);
        let read = options(&ctx);
        assert!(!read.marks && read.compact_labels && read.syntax);
        assert_eq!(read.font, 16);
        // Another context, such as a parallel test's, keeps its own.
        assert_eq!(options(&Context::default()), Options::default());
    }

    #[test]
    fn only_a_listed_font_and_a_full_path_are_valid() {
        assert!(Settings::default().valid());
        assert!(!Settings { diff_font: 40, ..Default::default() }.valid());
        assert!(!Settings { source_folder: Some("Code".into()), ..Default::default() }.valid());
        assert!(Settings { source_folder: Some("C:\\Code".into()), ..Default::default() }.valid());
    }

    #[test]
    fn diff_rows_keep_their_height_until_the_font_needs_more() {
        assert_eq!(diff_row_height(11), 22.0);
        assert_eq!(diff_row_height(13), 22.0);
        assert_eq!(diff_row_height(16), 25.0);
    }
}
