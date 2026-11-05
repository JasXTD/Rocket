//! Per-repository, read-only comparisons. A small raw-path listing is loaded
//! first; only the selected file's patch is prepared. Dropped receivers prevent
//! superseded selections from publishing into a newer view.
use crate::{
    app::*,
    backend::{Backend, user_error},
    diff::Patch,
    pages,
    style::*,
};
use eframe::egui::*;
use rocket_git::*;
use std::{
    collections::HashMap,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

pub enum CompareIntent {
    Open,
    Selected,
    Branch(String),
    Load,
    Swap,
    Select(usize),
    Retry,
    Cancel,
}

pub struct Loaded {
    pub base_input: String,
    pub target_input: String,
    pub comparison: Comparison,
    /// When the comparison was read; the result line says how long ago.
    pub at: Instant,
}

#[derive(Default)]
pub struct State {
    pub base: String,
    pub target: String,
    pub local_staged: Option<bool>,
    pub filter: String,
    pub loaded: Option<Loaded>,
    pub selected: Option<usize>,
    pub error: Option<String>,
    pub notice: Option<String>,
    pub cancelling: bool,
    pub show_files: bool,
    listing: Option<ReadTask<Loaded>>,
    content: Option<ReadTask<Patch>>,
    filtered: Option<(String, Vec<usize>)>,
    branches_at: u64,
    branch_choices: Vec<String>,
}

impl State {
    pub fn running(&self) -> bool {
        self.listing.is_some() || self.content.is_some()
    }
    pub fn listing(&self) -> bool {
        self.listing.is_some()
    }
    pub fn content_running(&self) -> bool {
        self.content.is_some()
    }
    pub fn changed_inputs(&self) -> bool {
        self.loaded.as_ref().is_some_and(|r| {
            r.base_input != self.base.trim() || r.comparison.local_staged != self.local_staged
                || (self.local_staged.is_none() && r.target_input != self.target.trim())
        })
    }
    pub fn selected_file(&self) -> Option<&ComparedFile> {
        self.loaded.as_ref()?.comparison.files.get(self.selected?)
    }
    pub fn stop(&mut self) {
        if let Some(t) = &self.listing {
            t.cancel();
        }
        if let Some(t) = &self.content {
            t.cancel();
        }
        self.cancelling = self.running();
    }
    pub fn leave(&mut self) {
        if self.running() {
            self.notice = Some("Comparison read cancelled when leaving this view.".into());
        }
        self.listing = None;
        self.content = None;
        self.cancelling = false;
    }
    fn load(&mut self, ctx: &Context, backend: Backend, repository: Repository) {
        self.leave();
        self.error = None;
        self.notice = None;
        let base_input = self.base.trim().to_owned();
        let target_input = self.target.trim().to_owned();
        let local_staged = self.local_staged;
        match crate::read_task::spawn(ctx, move |cancel| {
            backend.log.request("compare", |log| {
                let repo = repository.with_log(log.clone());
                let comparison = match local_staged {
                    Some(staged) => repo.compare_local(&base_input, staged, &cancel)?,
                    None => repo.compare_revisions(&base_input, &target_input, &cancel)?,
                };
                cancel.check()?;
                Ok(Loaded {
                    base_input,
                    target_input,
                    comparison,
                    at: Instant::now(),
                })
            })
        }) {
            Ok(t) => self.listing = Some(t),
            Err(e) => self.error = Some(error(&e)),
        }
    }
}

fn clear_patch(tab: &mut Tab) {
    tab.diff = None;
    tab.image.clear();
    tab.image_reload = None;
    tab.diff_tools.cancel();
    tab.patch = None;
    tab.patch_error = None;
    tab.patch_file = 0;
}

fn load_file(ctx: &Context, tab: &mut Tab, backend: Backend, index: usize) {
    let (Some(loaded), Some(snapshot)) = (&tab.compare.loaded, &tab.snapshot) else {
        return;
    };
    let Ok(options) = loaded.comparison.file_options(index) else {
        return;
    };
    let repository = snapshot.repository.clone();
    let comparison = loaded.comparison.clone();
    clear_patch(tab);
    let s = &mut tab.compare;
    s.content = None;
    s.selected = Some(index);
    s.cancelling = false;
    s.error = None;
    s.notice = None;
    match crate::read_task::spawn(ctx, move |cancel| {
        backend.log.request("compare-diff", |log| {
            let repo = repository.with_log(log.clone());
            repo.validate_comparison(&comparison, &cancel)?;
            let mut patch = Patch::parse(&repo.diff(&options, &cancel)?.patch);
            if patch.files.iter().any(|f| image_path(f.label.as_bytes())) || patch.needs_submodules() {
                let files = repo.file_comparisons(&options, &cancel)?;
                patch.load_submodules_for_files(&repo, &files, &cancel)?;
                let mut images: Vec<_> = files.into_iter().filter(|f| image_path(f.path().as_bytes())).collect();
                let guard = std::sync::Arc::new(comparison.clone());
                for image in &mut images { image.bind_comparison(guard.clone()); }
                patch.bind_images(images);
            }
            if patch.files.len() > 1 {
                patch.notice = Some(
                    "This path pair includes additional changes. Review each patch in the list."
                        .into(),
                );
            }
            cancel.check()?;
            repo.validate_comparison(&comparison, &cancel)?;
            Ok(patch)
        })
    }) {
        Ok(t) => s.content = Some(t),
        Err(e) => s.error = Some(error(&e)),
    }
}

impl Rocket {
    pub(crate) fn compare_intent(&mut self, intent: CompareIntent) {
        let Some(backend) = self.backend.clone() else {
            return;
        };
        if matches!(
            intent,
            CompareIntent::Open | CompareIntent::Selected | CompareIntent::Branch(_)
        ) {
            self.activity_history.close();
            self.close_browsing_page();
        }
        let Some(tab) = self.tabs.get_mut(self.active) else {
            return;
        };
        let Some(repo) = tab.snapshot.as_ref().map(|s| s.repository.clone()) else {
            return;
        };
        let mut run = false;
        match intent {
            CompareIntent::Open | CompareIntent::Selected | CompareIntent::Branch(_) => {
                let was_compare = tab.view == View::Compare;
                match intent {
                    CompareIntent::Selected => {
                        let selected: Vec<_> = tab
                            .commits()
                            .iter()
                            .filter(|c| tab.selected_commits.contains(&c.id))
                            .map(|c| c.id.to_string())
                            .collect();
                        if selected.len() == 2 {
                            tab.compare.local_staged = None;
                            tab.compare.base = selected[1].clone();
                            tab.compare.target = selected[0].clone();
                            run = true;
                        }
                    }
                    CompareIntent::Branch(reference) => {
                        tab.compare.local_staged = None;
                        tab.compare.base = "HEAD".into();
                        tab.compare.target = reference;
                        run = true;
                    }
                    _ => {}
                }
                if tab.compare.base.is_empty() && tab.compare.target.is_empty() {
                    let commit = tab
                        .selected_commit
                        .as_ref()
                        .and_then(|id| tab.commits().iter().find(|c| &c.id == id));
                    let base = commit
                        .and_then(|c| c.parents.first())
                        .map_or_else(|| "HEAD".into(), ToString::to_string);
                    let target = commit.map_or_else(|| "HEAD".into(), |c| c.id.to_string());
                    tab.compare.base = base;
                    tab.compare.target = target;
                    tab.compare.show_files = true;
                }
                tab.view = View::Compare;
                if !was_compare {
                    clear_patch(tab);
                    if !run && let Some(index) = tab.compare.selected {
                        load_file(&self.context, tab, backend.clone(), index);
                    }
                }
            }
            CompareIntent::Load => run = true,
            CompareIntent::Swap if tab.compare.local_staged.is_none() => {
                tab.compare.leave();
                std::mem::swap(&mut tab.compare.base, &mut tab.compare.target);
                tab.compare.error = None;
                tab.compare.notice = None;
            }
            CompareIntent::Select(index) if !tab.compare.listing() => {
                tab.compare.show_files = false;
                load_file(&self.context, tab, backend.clone(), index);
            }
            CompareIntent::Retry if !tab.compare.listing() => {
                if let Some(index) = tab.compare.selected {
                    load_file(&self.context, tab, backend.clone(), index);
                }
            }
            CompareIntent::Cancel => {
                tab.compare.stop();
                let _: rocket_git::Result<()> = backend
                    .log
                    .request("compare-cancel", |_| Err(Error::Cancelled));
            }
            _ => {}
        }
        if run {
            tab.compare.load(&self.context, backend, repo);
        }
    }

    pub(crate) fn poll_compare(&mut self, ctx: &Context) {
        for tab in &mut self.tabs {
            if tab.view != View::Compare {
                tab.compare.leave();
                continue;
            }
            let mut select_first = false;
            if let Some(result) = tab.compare.listing.as_mut().and_then(ReadTask::try_finish) {
                tab.compare.listing = None;
                match result {
                    Ok(loaded) => {
                        select_first = !loaded.comparison.files.is_empty();
                        tab.compare.loaded = Some(loaded);
                        tab.compare.selected = None;
                        tab.compare.show_files = true;
                        tab.compare.filter.clear();
                        tab.compare.filtered = None;
                        clear_patch(tab);
                    }
                    Err(Error::Cancelled) => {
                        tab.compare.notice =
                            Some("Comparison cancelled. Previous results retained.".into())
                    }
                    Err(e) => tab.compare.error = Some(error(&e)),
                }
            }
            if let Some(result) = tab.compare.content.as_mut().and_then(ReadTask::try_finish) {
                tab.compare.content = None;
                match result {
                    Ok(patch) => tab.patch = Some(patch),
                    Err(Error::Cancelled) => {
                        tab.compare.notice =
                            Some("File diff cancelled. Retry or select another file.".into())
                    }
                    Err(e) => tab.compare.error = Some(error(&e)),
                }
            }
            if select_first && let Some(backend) = &self.backend {
                load_file(ctx, tab, backend.clone(), 0);
            }
            if tab.compare.running() {
                ctx.request_repaint_after(Duration::from_millis(30));
            } else {
                tab.compare.cancelling = false;
            }
        }
    }
}

fn error(e: &Error) -> String {
    match e {
        Error::Git { code, .. } => format!(
            "Could not compare these revisions. Check both branch, tag or commit names and that their objects are available locally. [git_exit_{}]",
            code.map_or_else(|| "unknown".into(), |c| c.to_string())
        ),
        _ => user_error(e),
    }
}

pub fn change_label(change: Change) -> &'static str {
    match change {
        Change::Added => "Added",
        Change::Deleted => "Deleted",
        Change::Modified => "Modified",
        Change::Renamed => "Renamed",
        Change::Copied => "Copied",
        Change::TypeChanged => "Type changed",
        _ => "Changed",
    }
}

fn emit(intents: &mut Vec<Intent>, intent: CompareIntent) {
    intents.push(Intent::Compare(intent));
}
fn button(
    ui: &mut Ui,
    widgets: &mut HashMap<String, UiControl>,
    key: &str,
    label: &str,
    enabled: bool,
) -> bool {
    let r = ui.add_enabled(enabled, Button::new(label));
    pages::control(widgets, key, &r, ui.clip_rect());
    r.clicked()
}

/// Letters as the Changes list shows them.
pub(crate) fn letter(change: Change) -> char {
    match change {
        Change::Added => 'A',
        Change::Deleted => 'D',
        Change::Renamed | Change::Copied => 'R',
        Change::TypeChanged => 'T',
        _ => 'M',
    }
}

/// The result line's counts: each kind present, in a fixed order, with its letter.
pub(crate) fn kinds(files: &[ComparedFile]) -> Vec<(usize, &'static str, char)> {
    const ORDER: [(Change, &str); 6] = [
        (Change::Added, "added"),
        (Change::Modified, "modified"),
        (Change::Deleted, "deleted"),
        (Change::Renamed, "renamed"),
        (Change::Copied, "copied"),
        (Change::TypeChanged, "type changed"),
    ];
    let mut out: Vec<_> = ORDER
        .iter()
        .map(|&(change, word)| {
            (files.iter().filter(|f| f.change == change).count(), word, letter(change))
        })
        .filter(|(count, ..)| *count > 0)
        .collect();
    let other = files
        .iter()
        .filter(|f| !ORDER.iter().any(|(change, _)| *change == f.change))
        .count();
    if other > 0 {
        out.push((other, "changed", 'M'));
    }
    out
}

/// A patch that holds only Git's headers for one file: a rename or copy with no content
/// or mode change, which reads better as a card than as `rename from` lines.
fn header_only(patch: &Patch) -> bool {
    matches!(patch.files.as_slice(), [file] if file.image.is_none()
        && file.submodule.is_none()
        && file.lines.iter().all(|l| {
            matches!(l.kind, crate::diff::Kind::Header)
                && !l.text.starts_with("old mode")
                && !l.text.starts_with("new mode")
        }))
}

pub(crate) fn view(
    ui: &mut Ui,
    tab: &mut Tab,
    intents: &mut Vec<Intent>,
    widgets: &mut HashMap<String, UiControl>,
) {
    ui.set_clip_rect(ui.clip_rect().intersect(ui.available_rect_before_wrap()));
    let narrow = ui.available_width() < 820.0;
    let short = ui.available_height() < 380.0;
    if tab.compare.branches_at != tab.refresh_count {
        tab.compare.branch_choices = tab
            .snapshot
            .as_ref()
            .map(|s| {
                s.branches
                    .iter()
                    .filter_map(|b| std::str::from_utf8(&b.reference).ok().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default();
        tab.compare.branches_at = tab.refresh_count;
    }
    if short {
        ui.add_space(8.0);
    } else {
        pages::header(
            ui,
            Icon::Branch,
            "Compare",
            "Read only · nothing in the repository changes",
            |_| {},
        );
    }
    let previous = tab.compare.local_staged;
    bar(ui, &mut tab.compare, intents, widgets, narrow);
    if previous != tab.compare.local_staged {
        tab.compare.leave();
    }
    status(ui, &tab.compare, widgets, narrow);

    let s = &tab.compare;
    let Some(loaded) = &s.loaded else {
        if s.listing() {
            pages::loading(ui, "Reading the comparison…");
        } else if s.error.is_some() {
            message(ui, None, "Nothing to show until a comparison succeeds.", "", |_| ());
        } else {
            message(
                ui,
                Some((Icon::Branch, BLUE())),
                "Choose what to compare",
                "Pick a base and a target above. Branches, tags and commit IDs all work.",
                |_| (),
            );
        }
        return;
    };
    let total = loaded.comparison.files.len();
    if total == 0 {
        let target = match loaded.comparison.local_staged {
            Some(true) => "the staging area",
            Some(false) => "the working tree's tracked files",
            None => loaded.target_input.as_str(),
        };
        message(
            ui,
            Some((Icon::Check, GREEN())),
            "No differences",
            &format!("{} and {target} have the same content.", loaded.base_input),
            |_| (),
        );
        return;
    }
    // The file list and the diff share one card; a narrow window shows one at a time.
    Frame::new()
        .fill(BG())
        .stroke(Stroke::new(1.0, CARD_LINE()))
        .corner_radius(8)
        // Content stays inside the border, which the frame paints first.
        .inner_margin(1)
        .outer_margin(Margin { left: 24, right: 20, top: 0, bottom: 14 })
        .show(ui, |ui| {
            ui.set_min_size(ui.available_size());
            if narrow {
                view_tabs(ui, &mut tab.compare, total, widgets);
                if tab.compare.show_files {
                    file_list(ui, tab, intents, widgets);
                } else {
                    diff_side(ui, tab, intents, widgets);
                }
            } else {
                let left = CornerRadius { nw: 8, sw: 8, ne: 0, se: 0 };
                Panel::left(Id::new(("compare-files", tab.id)))
                    .default_size(300.0)
                    .size_range(200.0..=(ui.available_width() * 0.45))
                    .resizable(true)
                    .frame(Frame::new().fill(PANEL()).corner_radius(left))
                    .show(ui, |ui| file_list(ui, tab, intents, widgets));
                diff_side(ui, tab, intents, widgets);
            }
        });
}

/// What to compare: the mode, Base and Target (or the fixed local target), Swap between
/// them, and Compare, which Stop replaces while a read runs.
fn bar(
    ui: &mut Ui,
    s: &mut State,
    intents: &mut Vec<Intent>,
    widgets: &mut HashMap<String, UiControl>,
    narrow: bool,
) {
    Frame::new()
        .fill(PANEL())
        .stroke(Stroke::new(1.0, CARD_LINE()))
        .corner_radius(10)
        .inner_margin(Margin::symmetric(12, 10))
        .outer_margin(Margin { left: 24, right: 20, top: 0, bottom: 0 })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            if narrow {
                ui.spacing_mut().item_spacing.y = 8.0;
                modes(ui, s, widgets);
                fields(ui, s, intents, widgets);
            } else {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 10.0;
                    modes(ui, s, widgets);
                    let (line, _) = ui.allocate_exact_size(vec2(1.0, 22.0), Sense::hover());
                    ui.painter().rect_filled(line, 0.0, CARD_LINE());
                    fields(ui, s, intents, widgets);
                });
            }
        });
}

fn modes(ui: &mut Ui, s: &mut State, widgets: &mut HashMap<String, UiControl>) {
    const MODES: [(&str, &str, Option<bool>); 3] = [
        ("compare-mode-revision", "Revisions", None),
        ("compare-mode-index", "Staging area", Some(true)),
        ("compare-mode-working", "Working tree", Some(false)),
    ];
    let items = MODES.map(|(_, label, value)| (label, s.local_staged == value));
    for (r, (key, _, value)) in pages::segments(ui, &items).iter().zip(MODES) {
        pages::control(widgets, key, r, ui.clip_rect());
        if r.clicked() {
            s.local_staged = value;
        }
    }
}

fn fields(
    ui: &mut Ui,
    s: &mut State,
    intents: &mut Vec<Intent>,
    widgets: &mut HashMap<String, UiControl>,
) {
    let local = s.local_staged;
    let mut enter = false;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let action = 84.0;
        let field = ((ui.available_width() - action - 30.0 - 3.0 * 8.0) / 2.0).max(110.0);
        let base = RefField { key: "compare-base", label: "BASE", hint: "Branch, tag or commit", invalid: false };
        ref_field(ui, widgets, base, &mut s.base, &s.branch_choices, field, &mut enter);
        match local {
            Some(staged) => {
                let (rect, _) = ui.allocate_exact_size(vec2(30.0, 30.0), Sense::hover());
                arrow(ui.painter(), rect.center(), false, FAINT());
                fixed_target(ui, staged, field);
            }
            None => {
                if swap_button(ui, widgets, "compare-swap", !s.cancelling) {
                    emit(intents, CompareIntent::Swap);
                }
                let target = RefField { key: "compare-target", label: "TARGET", hint: "Branch, tag or commit", invalid: false };
                ref_field(ui, widgets, target, &mut s.target, &s.branch_choices, field, &mut enter);
            }
        }
        if s.running() {
            let label = if s.cancelling { "Stopping" } else { "Stop" };
            let r = ui.add_enabled(!s.cancelling, Button::new(label).min_size(vec2(action, 30.0)));
            pages::control(widgets, "compare-cancel", &r, ui.clip_rect());
            if r.clicked() {
                emit(intents, CompareIntent::Cancel);
            }
        } else {
            let valid = !s.base.trim().is_empty() && (local.is_some() || !s.target.trim().is_empty());
            let compare = Button::new(RichText::new("Compare").color(ACCENT_INK()).strong())
                .fill(BLUE())
                .min_size(vec2(action, 30.0));
            let r = ui.add_enabled(valid, compare);
            pages::control(widgets, "compare-load", &r, ui.clip_rect());
            if r.clicked() || enter && valid {
                emit(intents, CompareIntent::Load);
            }
        }
    });
}

/// What a Base or Target field shows besides its text: its MCP key, its label, the hint
/// while empty, and whether its name was found.
pub(crate) struct RefField<'a> {
    pub key: &'a str,
    pub label: &'a str,
    pub hint: &'a str,
    pub invalid: bool,
}

/// Base or Target as one field: its label, the name and a bare ▾ for the branch list.
/// Returns whether a name was chosen from the list; typing shows in the text's response.
pub(crate) fn ref_field(
    ui: &mut Ui,
    widgets: &mut HashMap<String, UiControl>,
    field: RefField<'_>,
    value: &mut String,
    choices: &[String],
    width: f32,
    enter: &mut bool,
) -> bool {
    let RefField { key, label, hint, invalid } = field;
    let (rect, _) = ui.allocate_exact_size(vec2(width, 30.0), Sense::hover());
    ui.painter().rect_filled(rect, 7.0, BG());
    let inside = Rect::from_min_max(rect.min + vec2(10.0, 0.0), rect.max - vec2(3.0, 0.0));
    let mut field = ui.new_child(UiBuilder::new().max_rect(inside).layout(Layout::left_to_right(Align::Center)));
    field.spacing_mut().item_spacing.x = 6.0;
    field.label(RichText::new(label).font(semibold(10.5)).extra_letter_spacing(0.7).color(FAINT()));
    let picker = 24.0;
    let r = field.add_sized(
        [(field.available_width() - picker - 6.0).max(40.0), 24.0],
        TextEdit::singleline(value)
            .char_limit(4096)
            .frame(Frame::NONE)
            .hint_text(hint),
    );
    *enter |= r.lost_focus() && field.input(|i| i.key_pressed(Key::Enter));
    pages::control(widgets, key, &r, ui.clip_rect());
    let mut picked = false;
    field.scope(|ui| {
        // The list opens from a bare ▾ inside the field, framed only while hovered.
        let visuals = &mut ui.visuals_mut().widgets;
        visuals.inactive.weak_bg_fill = Color32::TRANSPARENT;
        for state in [&mut visuals.inactive, &mut visuals.hovered, &mut visuals.active, &mut visuals.open] {
            state.bg_stroke = Stroke::NONE;
        }
        ui.spacing_mut().combo_width = picker;
        if let Some(chosen) = crate::branch_picker::pick(ui, widgets, &format!("{key}-pick"), "", choices, true) {
            *value = chosen;
            picked = true;
        }
    });
    let edge = if invalid { RED() } else if r.has_focus() { BLUE() } else { BORDER() };
    ui.painter().rect_stroke(rect, 7.0, Stroke::new(1.0, edge), StrokeKind::Inside);
    picked
}

/// The local target, which can't be typed: the staging area or the working tree.
fn fixed_target(ui: &mut Ui, staged: bool, width: f32) {
    let (rect, r) = ui.allocate_exact_size(vec2(width, 30.0), Sense::hover());
    ui.painter().rect_filled(rect, 7.0, RAISED());
    let y = rect.center().y;
    let label = ui.painter().text(
        pos2(rect.left() + 10.0, y),
        Align2::LEFT_CENTER,
        "TARGET",
        semibold(10.5),
        FAINT(),
    );
    ui.painter().text(
        pos2(label.right() + 8.0, y),
        Align2::LEFT_CENTER,
        if staged { "Staging area" } else { "Working tree" },
        FontId::proportional(13.0),
        MUTED(),
    );
    r.on_hover_text(if staged {
        "What is staged now (the index)"
    } else {
        "Tracked files in the working tree; untracked files aren't included"
    });
}

/// A small arrow pointing right, or left when `back`.
fn arrow(painter: &Painter, at: Pos2, back: bool, color: Color32) {
    let stroke = Stroke::new(1.5, color);
    let d = if back { -1.0 } else { 1.0 };
    painter.line_segment([at - vec2(6.0 * d, 0.0), at + vec2(6.0 * d, 0.0)], stroke);
    painter.line_segment([at + vec2(3.0 * d, -3.0), at + vec2(6.0 * d, 0.0)], stroke);
    painter.line_segment([at + vec2(3.0 * d, 3.0), at + vec2(6.0 * d, 0.0)], stroke);
}

/// Swap as two arrows between the fields.
pub(crate) fn swap_button(ui: &mut Ui, widgets: &mut HashMap<String, UiControl>, key: &str, enabled: bool) -> bool {
    let r = ui
        .add_enabled(enabled, Button::new("").min_size(vec2(30.0, 30.0)))
        .on_hover_text("Swap base and target");
    r.widget_info(|| WidgetInfo::labeled(WidgetType::Button, enabled, "Swap base and target"));
    let color = if enabled { TEXT() } else { FAINT() };
    let c = r.rect.center();
    arrow(ui.painter(), c - vec2(0.0, 3.5), false, color);
    arrow(ui.painter(), c + vec2(0.0, 3.5), true, color);
    pages::control(widgets, key, &r, ui.clip_rect());
    r.clicked()
}

/// Under the bar: what was compared and what changed, or that the fields no longer match
/// it; then any error or notice.
fn status(ui: &mut Ui, s: &State, widgets: &mut HashMap<String, UiControl>, narrow: bool) {
    Frame::new()
        .inner_margin(Margin { left: 26, right: 22, top: 9, bottom: 9 })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            if let Some(loaded) = &s.loaded {
                if s.listing() {
                    let text = if s.cancelling { "Stopping the comparison…" } else { "Reading the comparison…" };
                    pages::busy_line(ui, text);
                } else if s.changed_inputs() {
                    ui.label(
                        RichText::new("Not compared yet · the files below are from the last comparison")
                            .size(12.5)
                            .color(AMBER()),
                    );
                } else {
                    summary(ui, loaded, widgets, narrow);
                }
            }
            if let Some(e) = &s.error {
                pages::callout(ui, RED(), e);
            } else if let Some(n) = &s.notice {
                ui.label(RichText::new(n).size(12.0).color(HINT()));
            }
        });
}

fn summary(ui: &mut Ui, loaded: &Loaded, widgets: &mut HashMap<String, UiControl>, narrow: bool) {
    let c = &loaded.comparison;
    let age = loaded.at.elapsed().as_secs();
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let scope = if c.local_staged.is_some() { "Tracked files only" } else { "Direct comparison" };
    let when = format!("{scope} · compared {}", ago(age, now.saturating_sub(age) as i64));
    // Keep "compared … ago" current while the page is idle.
    ui.ctx().request_repaint_after(Duration::from_secs(30));
    let line = |ui: &mut Ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        sha(ui, widgets, "compare-copy-base", &c.base, &loaded.base_input);
        ui.label(RichText::new("→").color(HINT()));
        match c.local_staged {
            Some(staged) => {
                ui.label(RichText::new(if staged { "staging area" } else { "working tree" }).color(MUTED()));
            }
            None => sha(ui, widgets, "compare-copy-target", &c.target, &loaded.target_input),
        }
        let n = c.files.len();
        if n == 0 {
            ui.label(RichText::new("no differences").color(MUTED()));
            return;
        }
        ui.label(RichText::new(format!("{n} file{}", if n == 1 { "" } else { "s" })).font(semibold(13.0)));
        for (count, word, kind) in kinds(&c.files) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 5.0;
                let (dot, _) = ui.allocate_exact_size(vec2(7.0, 7.0), Sense::hover());
                ui.painter().circle_filled(dot.center(), 3.5, status_color(kind));
                ui.label(RichText::new(format!("{count} {word}")).size(12.5).color(MUTED()));
            });
        }
    };
    if narrow {
        ui.horizontal_wrapped(line);
    } else {
        ui.horizontal(|ui| {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(RichText::new(when).size(12.0).color(HINT()));
                ui.with_layout(Layout::left_to_right(Align::Center), line);
            });
        });
    }
}

/// A commit's short ID; clicking it copies the full one.
pub(crate) fn sha(ui: &mut Ui, widgets: &mut HashMap<String, UiControl>, key: &str, id: &ObjectId, name: &str) {
    let text = RichText::new(&id.as_str()[..7]).monospace().size(12.0).color(BLUE());
    let chip = Button::new(text)
        .small()
        .fill(BLUE().gamma_multiply(0.12))
        .stroke(Stroke::NONE)
        .corner_radius(4)
        .min_size(vec2(0.0, 20.0));
    let r = ui.add(chip).on_hover_text(format!("{name}\n{id}\nClick to copy the full ID"));
    pages::control(widgets, key, &r, ui.clip_rect());
    if r.clicked() {
        ui.ctx().copy_text(id.to_string());
    }
}

/// A message centered in the rest of the pane: an optional icon, a title, a hint and
/// whatever `below` adds.
pub(crate) fn message(ui: &mut Ui, image: Option<(Icon, Color32)>, title: &str, hint: &str, below: impl FnOnce(&mut Ui)) {
    let size = ui.available_size();
    let height = if size.y.is_finite() && size.y < 4000.0 { size.y.max(140.0) } else { 200.0 };
    let group = image.map_or(0.0, |_| 50.0) + 20.0 + if hint.is_empty() { 0.0 } else { 22.0 };
    ui.allocate_ui_with_layout(vec2(size.x, height), Layout::top_down(Align::Center), |ui| {
        ui.set_min_size(vec2(size.x, height));
        ui.add_space(((height - group) / 2.0 - 16.0).max(12.0));
        if let Some((image, color)) = image {
            let (tile, _) = ui.allocate_exact_size(vec2(40.0, 40.0), Sense::hover());
            ui.painter().circle_filled(tile.center(), 20.0, color.gamma_multiply(0.14));
            icon(ui.painter(), Rect::from_center_size(tile.center(), vec2(18.0, 18.0)), image, color);
            ui.add_space(4.0);
        }
        ui.label(RichText::new(title).size(14.0).color(TEXT()));
        if !hint.is_empty() {
            ui.label(RichText::new(hint).size(12.5).color(HINT()));
        }
        below(ui);
    });
}

/// Files | Diff in a narrow window. The toggle id moves to whichever tab switches views.
fn view_tabs(ui: &mut Ui, s: &mut State, total: usize, widgets: &mut HashMap<String, UiControl>) {
    Frame::new()
        .inner_margin(Margin { left: 10, right: 10, top: 8, bottom: 2 })
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let files = format!("Files {total}");
                let r = pages::segments(ui, &[(files.as_str(), s.show_files), ("Diff", !s.show_files)]);
                pages::control(widgets, "compare-toggle-files", &r[usize::from(s.show_files)], ui.clip_rect());
                if r[0].clicked() {
                    s.show_files = true;
                }
                if r[1].clicked() {
                    s.show_files = false;
                }
            });
        });
}

fn diff_side(
    ui: &mut Ui,
    tab: &mut Tab,
    intents: &mut Vec<Intent>,
    widgets: &mut HashMap<String, UiControl>,
) {
    if tab.compare.content_running() {
        pages::loading(ui, "Loading the file's changes…");
        return;
    }
    let Some(patch) = &tab.patch else {
        if tab.compare.selected.is_some() {
            let listing = tab.compare.listing();
            message(ui, None, "This file's changes didn't load.", "", |ui| {
                ui.add_space(6.0);
                if button(ui, widgets, "compare-retry", "Retry", !listing) {
                    emit(intents, CompareIntent::Retry);
                }
            });
        } else {
            message(ui, None, "Select a file to see its changes.", "", |_| ());
        }
        return;
    };
    let moved = tab
        .compare
        .selected_file()
        .filter(|f| matches!(f.change, Change::Renamed | Change::Copied));
    if let Some(file) = moved
        && header_only(patch)
    {
        let file = file.clone();
        rename_card(ui, &file);
        return;
    }
    let multiple = patch.files.len() > 1;
    if let Some(action) = crate::patch_view::patch_view(ui, tab, multiple, false, widgets) {
        intents.push(action);
    }
}

/// A rename or copy with no content change: where the file moved, not Git's headers.
pub(crate) fn rename_card(ui: &mut Ui, file: &ComparedFile) {
    let copied = file.change == Change::Copied;
    let from = file
        .original_path
        .as_ref()
        .map(|p| crate::text::raw_path_label(p.as_bytes()))
        .unwrap_or_default();
    let to = crate::text::raw_path_label(file.path.as_bytes());
    let size = ui.available_size();
    let height = if size.y.is_finite() && size.y < 4000.0 { size.y.max(160.0) } else { 220.0 };
    ui.allocate_ui_with_layout(vec2(size.x, height), Layout::top_down(Align::Center), |ui| {
        ui.set_min_size(vec2(size.x, height));
        ui.add_space(((height - 130.0) / 2.0 - 16.0).max(16.0));
        Frame::new()
            .fill(VIOLET().gamma_multiply(0.06))
            .stroke(Stroke::new(1.0, VIOLET().gamma_multiply(0.35)))
            .corner_radius(9)
            .inner_margin(Margin::symmetric(16, 14))
            .show(ui, |ui| {
                let width = (size.x - 80.0).clamp(220.0, 560.0);
                ui.set_width(width);
                // The card reads left to right inside the centered page.
                ui.with_layout(Layout::top_down(Align::Min), |ui| {
                    ui.spacing_mut().item_spacing.y = 8.0;
                    ui.horizontal(|ui| {
                        let (badge, _) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
                        status_badge(ui.painter(), badge.center(), 'R');
                        let title = if copied { "Copied · no content changes" } else { "Renamed · no content changes" };
                        ui.label(RichText::new(title).font(semibold(13.5)));
                    });
                    for (label, path, color) in [("From", &from, MUTED()), ("To", &to, TEXT())] {
                        ui.horizontal(|ui| {
                            let (cell, _) = ui.allocate_exact_size(vec2(40.0, 18.0), Sense::hover());
                            ui.painter().text(cell.left_center(), Align2::LEFT_CENTER, label, FontId::proportional(12.0), HINT());
                            ui.add(Label::new(RichText::new(path).monospace().size(12.0).color(color)).truncate());
                        });
                    }
                    let similar = file.similarity.map_or_else(String::new, |n| format!("{n}% similar · "));
                    ui.label(
                        RichText::new(format!("{similar}the file's content is the same on both sides"))
                            .size(12.0)
                            .color(HINT()),
                    );
                });
            });
    });
}

/// Changed files as rows like the Changes list: a letter, the name, then its folder or
/// where it came from.
fn file_list(
    ui: &mut Ui,
    tab: &mut Tab,
    intents: &mut Vec<Intent>,
    widgets: &mut HashMap<String, UiControl>,
) {
    let Some(total) = tab.compare.loaded.as_ref().map(|l| l.comparison.files.len()) else {
        return;
    };
    Frame::new()
        .inner_margin(Margin { left: 10, right: 10, top: 10, bottom: 6 })
        .show(ui, |ui| {
            let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::hover());
            ui.painter().rect_filled(rect, 6.0, BG());
            let glass = Rect::from_center_size(pos2(rect.left() + 15.0, rect.center().y), vec2(14.0, 14.0));
            icon(ui.painter(), glass, Icon::Search, FAINT());
            let inside = Rect::from_min_max(pos2(rect.left() + 28.0, rect.top()), pos2(rect.right() - 6.0, rect.bottom()));
            let mut field = ui.new_child(UiBuilder::new().max_rect(inside).layout(Layout::left_to_right(Align::Center)));
            let hint = format!("Filter {total} file{}", if total == 1 { "" } else { "s" });
            let r = field.add(
                TextEdit::singleline(&mut tab.compare.filter)
                    .frame(Frame::NONE)
                    .hint_text(hint)
                    .desired_width(inside.width()),
            );
            pages::control(widgets, "compare-filter", &r, ui.clip_rect());
            let edge = if r.has_focus() { BLUE() } else { BORDER() };
            ui.painter().rect_stroke(rect, 6.0, Stroke::new(1.0, edge), StrokeKind::Inside);
        });
    let Some(loaded) = &tab.compare.loaded else {
        return;
    };
    let filter = tab.compare.filter.to_lowercase();
    if tab
        .compare
        .filtered
        .as_ref()
        .is_none_or(|(query, _)| query != &filter)
    {
        let visible: Vec<_> = loaded
            .comparison
            .files
            .iter()
            .enumerate()
            .filter(|(_, f)| {
                filter.is_empty()
                    || f.path.display_lossy().to_lowercase().contains(&filter)
                    || f.original_path
                        .as_ref()
                        .is_some_and(|p| p.display_lossy().to_lowercase().contains(&filter))
            })
            .map(|(index, _)| index)
            .collect();
        tab.compare.filtered = Some((filter, visible));
    }
    let visible = &tab.compare.filtered.as_ref().unwrap().1;
    if visible.is_empty() {
        ui.add_space(8.0);
        ui.vertical_centered(|ui| ui.label(RichText::new("No files match this filter.").color(FAINT())));
    }
    let enabled = !tab.compare.listing();
    ui.spacing_mut().item_spacing.y = 0.0;
    let out = ScrollArea::vertical()
        .id_salt(("compare-file-scroll", tab.id))
        .auto_shrink([false, false])
        .show_rows(ui, 30.0, visible.len(), |ui, rows| {
            for index in rows {
                let i = visible[index];
                let file = &loaded.comparison.files[i];
                let path = crate::text::raw_path_label(file.path.as_bytes());
                let (dir, name) = path.rsplit_once('/').unwrap_or(("", path.as_str()));
                let detail = match &file.original_path {
                    Some(old) => format!(
                        "{} {}",
                        if file.change == Change::Copied { "copy of" } else { "from" },
                        crate::text::raw_path_label(old.as_bytes())
                    ),
                    None => dir.to_owned(),
                };
                let r = ui
                    .add_enabled_ui(enabled, |ui| {
                        ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click()).1
                    })
                    .inner;
                let what = match &file.original_path {
                    Some(_) => format!("{} {detail}", change_label(file.change)),
                    None => change_label(file.change).to_owned(),
                };
                let r = r.on_hover_text(format!("{path}\n{what}"));
                r.widget_info(|| WidgetInfo::labeled(WidgetType::Button, enabled, format!("{path}: {what}")));
                let row = r.rect.shrink2(vec2(6.0, 1.0));
                row_background(ui.painter(), row, tab.compare.selected == Some(i), r.hovered());
                status_badge(ui.painter(), pos2(row.left() + 15.0, row.center().y), letter(file.change));
                let text = Rect::from_min_max(pos2(row.left() + 32.0, row.top()), pos2(row.right() - 6.0, row.bottom()));
                let ink = if enabled { TEXT() } else { FAINT() };
                let galley = ui.painter().layout_no_wrap(name.to_owned(), FontId::proportional(13.0), ink);
                let width = galley.size().x.min(text.width());
                let top = row.center().y - galley.size().y / 2.0;
                ui.painter().with_clip_rect(text.intersect(ui.clip_rect())).galley(pos2(text.left(), top), galley, ink);
                if file.change == Change::Deleted {
                    let stroke = Stroke::new(1.0, RED().gamma_multiply(0.7));
                    ui.painter().hline(text.left()..=text.left() + width, row.center().y, stroke);
                }
                let after = text.left() + width + 8.0;
                if !detail.is_empty() && after < text.right() - 20.0 {
                    let rest = Rect::from_min_max(pos2(after, row.top()), pos2(text.right(), row.bottom()));
                    clipped_text(ui, rest, &detail, 12.0, FAINT());
                }
                pages::control(widgets, format!("compare-file-{i}"), &r, ui.clip_rect());
                if r.clicked() {
                    emit(intents, CompareIntent::Select(i));
                }
            }
        });
    let rect = out.inner_rect.intersect(ui.clip_rect());
    if rect.is_positive() {
        widgets.insert(
            "compare-files-scroll".into(),
            UiControl {
                rect,
                enabled: true,
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: &str, change: Change, original: Option<&str>) -> ComparedFile {
        ComparedFile {
            path: RepoPath::new(path.as_bytes().to_vec()).unwrap(),
            original_path: original.map(|p| RepoPath::new(p.as_bytes().to_vec()).unwrap()),
            change,
            similarity: original.map(|_| 100),
        }
    }

    #[test]
    fn the_result_line_counts_each_kind_in_order() {
        let files = [
            file("b.rs", Change::Modified, None),
            file("a.md", Change::Added, None),
            file("new.rs", Change::Renamed, Some("old.rs")),
            file("c.md", Change::Added, None),
            file("gone.md", Change::Deleted, None),
        ];
        assert_eq!(
            kinds(&files),
            [(2, "added", 'A'), (1, "modified", 'M'), (1, "deleted", 'D'), (1, "renamed", 'R')]
        );
        assert!(kinds(&[]).is_empty());
    }

    #[test]
    fn only_a_header_only_rename_becomes_a_card() {
        let rename = b"diff --git a/old.rs b/new.rs\nsimilarity index 100%\nrename from old.rs\nrename to new.rs\n";
        assert!(header_only(&Patch::parse(rename)));
        let chmod = b"diff --git a/old.rs b/new.rs\nold mode 100644\nnew mode 100755\nsimilarity index 100%\nrename from old.rs\nrename to new.rs\n";
        assert!(!header_only(&Patch::parse(chmod)), "a mode change stays a diff");
        let edited = b"diff --git a/old.rs b/new.rs\nsimilarity index 80%\nrename from old.rs\nrename to new.rs\n--- a/old.rs\n+++ b/new.rs\n@@ -1 +1 @@\n-a\n+b\n";
        assert!(!header_only(&Patch::parse(edited)), "content changes stay a diff");
    }
}
