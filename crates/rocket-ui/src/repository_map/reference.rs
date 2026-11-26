use super::*;
use crate::tree::{Content, path_label};
use std::{
    collections::BTreeSet,
    ops::Range,
    time::{Duration, Instant},
};

/// The list beside the open reference when there are several and the window is wide.
const LIST_WIDTH: f32 = 300.;
/// A list row: the file and line, then the kind and what it points to.
const ROW: f32 = 52.;
/// How long Copy location reads Copied.
const COPIED_FOR: Duration = Duration::from_millis(1600);

impl MapPage {
    pub(super) fn reference_sites(
        &self,
        commit: &ObjectId,
        path: &[u8],
    ) -> Option<&BTreeSet<AssetReference>> {
        let data = self.data.as_ref()?;
        if data.tip.as_ref() != Some(commit) {
            return None;
        }
        let index = match data.kind {
            MapKind::Modules => &data.modules.as_ref()?.references,
            MapKind::Code if !self.alternate => {
                let links = self.code.links.as_ref()?;
                if links.tip.as_ref() != Some(commit) {
                    return None;
                }
                &links.references
            }
            _ => return None,
        };
        index.for_file(path)
    }
}

pub struct ReferenceView {
    pub commit: ObjectId,
    pub sites: Vec<AssetReference>,
    pub selected: Option<usize>,
    pub error: Option<String>,
    /// A narrow window shows the list in place of the open reference.
    pub listing: bool,
    /// The file picked in the map; a narrow window's list is titled with it.
    file: Vec<u8>,
    /// The map header's words for this snapshot, such as "Snapshot of main · 4a59df78".
    snapshot: String,
    /// The error is the line missing from this commit, not a failed read.
    missing: bool,
    task: Option<ReadTask<Content>>,
    content: Option<Content>,
    scroll: bool,
    copied: Option<Instant>,
}

/// What the header's buttons asked for this frame.
#[derive(Default)]
struct Header {
    back: bool,
    step: Option<usize>,
    copy: bool,
}

impl ReferenceView {
    pub(super) fn new(commit: ObjectId, sites: Vec<AssetReference>, file: Vec<u8>, snapshot: String) -> Self {
        Self {
            commit,
            sites,
            selected: None,
            error: None,
            listing: false,
            file,
            snapshot,
            missing: false,
            task: None,
            content: None,
            scroll: false,
            copied: None,
        }
    }

    pub fn loading(&self) -> bool {
        self.task.is_some()
    }

    pub fn focused_text(&self) -> Option<&str> {
        let content = self.content.as_ref()?;
        let line = self.sites.get(self.selected?)?.line as usize;
        content
            .lines
            .get(line.checked_sub(content.first)?)
            .map(String::as_str)
    }

    /// The text marked in the reference line, as the viewer shows it.
    pub fn marked_text(&self) -> Option<String> {
        let content = self.content.as_ref()?;
        let index = (self.sites.get(self.selected?)?.line as usize).checked_sub(content.first)?;
        let range = content.marks.get(&index)?.first()?;
        Some(content.lines[index].chars().skip(range.start).take(range.len()).collect())
    }

    /// Only the lines around the reference were read: the file is too large to show whole.
    pub fn window(&self) -> bool {
        self.content.as_ref().is_some_and(|c| c.window)
    }

    pub(super) fn open(&mut self, index: usize, ctx: &Context, backend: Backend, repo: Repository) {
        let Some(site) = self.sites.get(index) else {
            return;
        };
        let (source, line) = (site.source.clone(), u64::from(site.line));
        let revision = FileSearchSource::Commit(self.commit.clone());
        self.selected = Some(index);
        self.listing = false;
        self.task = None;
        self.content = None;
        self.error = None;
        self.missing = false;
        self.copied = None;
        self.scroll = true;
        match crate::read_task::spawn(ctx, move |cancel| {
            backend.log.request("map-open-reference", |log| {
                let blob = repo
                    .with_log(log.clone())
                    .search_file(&revision, &source, line, &cancel)?;
                crate::tree::prepare_search(blob, line, &HashMap::new(), &cancel)
            })
        }) {
            Ok(task) => self.task = Some(task),
            Err(e) => self.error = Some(user_error(&e)),
        }
    }

    pub(super) fn poll(&mut self) {
        if let Some(result) = self.task.as_mut().and_then(ReadTask::try_finish) {
            self.task = None;
            match result {
                Ok(content) => self.loaded(content),
                Err(e) => self.error = Some(user_error(&e)),
            }
        }
    }

    /// The open reference's file arrived: mark where its line names the target, or say
    /// the line isn't in this commit.
    fn loaded(&mut self, mut content: Content) {
        if let Some(site) = self.selected.and_then(|i| self.sites.get(i))
            && let Some(index) = (site.line as usize).checked_sub(content.first)
            && let Some(range) = content.lines.get(index).and_then(|text| mark(text, &site.target))
        {
            content.marks.insert(index, vec![range]);
        }
        self.content = Some(content);
        if self.focused_text().is_none() {
            self.missing = true;
            self.error = Some(tr("This reference line is unavailable in the selected commit.").into());
        }
    }

    /// True when the view should close. Closing drops its read; the map keeps its
    /// selection and zoom underneath, so they show again as they were.
    pub(super) fn show(
        &mut self,
        ui: &mut Ui,
        intents: &mut Vec<Intent>,
        widgets: &mut HashMap<String, UiControl>,
    ) -> bool {
        let narrow = ui.available_width() < COMPACT;
        let several = self.sites.len() > 1;
        let listing = |v: &Self| several && (v.selected.is_none() || narrow && v.listing);
        // In a narrow window, back from one of several references is back to the list.
        let to_list = narrow && several && !listing(self);
        let asked = self.header(ui, narrow, listing(self), to_list, widgets);
        if asked.back || ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
            if !to_list {
                return true;
            }
            self.listing = true;
        }
        if let Some(index) = asked.step {
            emit(intents, MapIntent::SelectReference(index));
        }
        if asked.copy
            && let Some(site) = self.selected.and_then(|i| self.sites.get(i))
        {
            ui.ctx().copy_text(format!("{}:{}", String::from_utf8_lossy(&site.source), site.line));
            self.copied = Some(Instant::now());
            ui.ctx().request_repaint_after(COPIED_FOR);
        }
        let body = ui.available_rect_before_wrap();
        if listing(self) {
            let inner = Rect::from_min_max(body.min + vec2(16., 14.), body.max - vec2(16., 16.));
            ui.scope_builder(UiBuilder::new().max_rect(inner), |ui| self.list(ui, true, intents, widgets));
        } else if several && !narrow {
            let list = Rect::from_min_max(body.min, pos2(body.left() + LIST_WIDTH, body.bottom()));
            ui.painter().rect_filled(list, 0., PANEL());
            ui.painter().vline(list.right() - 0.5, list.y_range(), Stroke::new(1., HAIRLINE()));
            ui.scope_builder(UiBuilder::new().max_rect(list), |ui| self.list(ui, false, intents, widgets));
            let rest = Rect::from_min_max(pos2(list.right(), body.top()), body.max);
            self.source(ui, rest, widgets);
        } else {
            self.source(ui, body, widgets);
        }
        ui.advance_cursor_after_rect(body);
        false
    }

    /// Back on the left, then what is open (or, for a narrow list, the file and how many
    /// references), then Copy location and the steps through several.
    fn header(
        &self,
        ui: &mut Ui,
        narrow: bool,
        listing: bool,
        to_list: bool,
        widgets: &mut HashMap<String, UiControl>,
    ) -> Header {
        let mut out = Header::default();
        let site = self.selected.and_then(|i| self.sites.get(i)).filter(|_| !listing);
        let copied = self.copied.is_some_and(|at| at.elapsed() < COPIED_FOR);
        let frame = Frame::new()
            .fill(PANEL())
            .inner_margin(Margin { left: 10, right: 14, top: 10, bottom: 10 })
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 10.;
                    let (id, label) = if to_list {
                        ("map-reference-list", tr("References"))
                    } else {
                        ("map-reference-close", tr("Map"))
                    };
                    let r = pages::tool(ui, Icon::ArrowLeft, label, true);
                    register(ui, widgets, id, &r);
                    out.back = r.clicked();
                    let (rule, _) = ui.allocate_exact_size(vec2(1., 28.), Sense::hover());
                    ui.painter().rect_filled(rule, 0., HAIRLINE());
                    let (tile, _) = ui.allocate_exact_size(vec2(32., 32.), Sense::hover());
                    ui.painter().rect_filled(tile, 8., RAISED());
                    icon(ui.painter(), Rect::from_center_size(tile.center(), vec2(16., 16.)), Icon::Link, BLUE());
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = 4.;
                        if let (Some(index), true) = (self.selected.filter(|_| site.is_some()), self.sites.len() > 1) {
                            let last = self.sites.len() - 1;
                            for (id, image, hint, to) in [
                                ("map-reference-next", Icon::ArrowDown, tr("Next reference"), (index < last).then(|| index + 1)),
                                ("map-reference-previous", Icon::ArrowUp, tr("Previous reference"), index.checked_sub(1)),
                            ] {
                                let r = step_button(ui, image, to.is_some()).on_hover_text(hint);
                                register(ui, widgets, id, &r);
                                if r.clicked() {
                                    out.step = to;
                                }
                            }
                            ui.add_space(4.);
                            let of = trf!("{number} of {total}", number = index + 1, total = self.sites.len());
                            ui.label(RichText::new(of).size(12.).color(HINT()));
                            ui.add_space(8.);
                        }
                        if site.is_some() {
                            let (image, label) =
                                if copied { (Icon::Check, tr("Copied")) } else { (Icon::Copy, tr("Copy location")) };
                            let r = if narrow {
                                crate::chrome::icon_button(ui, image, true, false, 28.).on_hover_text(label)
                            } else {
                                pages::tool(ui, image, label, true)
                            };
                            register(ui, widgets, "map-reference-copy", &r);
                            out.copy = r.clicked();
                            ui.add_space(6.);
                        }
                        ui.with_layout(Layout::left_to_right(Align::Center), |ui| self.title(ui, site, listing));
                    });
                });
            });
        let rect = frame.response.rect;
        ui.painter().hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1., HAIRLINE()));
        out
    }

    /// Two lines: the file that makes the reference → the file it points to, then the
    /// kind, the line and the snapshot. A narrow list shows the picked file and a count.
    fn title(&self, ui: &mut Ui, site: Option<&AssetReference>, listing: bool) {
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 4.;
            let mut job = text::LayoutJob::default();
            let (hover, detail) = match site.filter(|_| !listing) {
                Some(site) => {
                    let (source, target) = (path_label(&site.source), path_label(&site.target));
                    crate::tree::append_path(&mut job, &source, 14.);
                    job.append("  →  ", 0., TextFormat::simple(FontId::proportional(14.), FAINT()));
                    crate::tree::append_path(&mut job, &target, 14.);
                    let line = trf!("Line {line}", line = pages::number_text(site.line as usize));
                    (format!("{source}:{} → {target}", site.line), format!("{line} · {}", self.snapshot))
                }
                None => {
                    let file = path_label(&self.file);
                    crate::tree::append_path(&mut job, &file, 14.);
                    let count = trn!(self.sites.len(), "{n} reference", "{n} references");
                    (file, format!("{count} · {}", self.snapshot))
                }
            };
            ui.add(Label::new(job).truncate()).on_hover_text(hover);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.;
                if let Some(site) = site.filter(|_| !listing) {
                    kind_chip(ui, site.kind);
                }
                ui.add(Label::new(RichText::new(detail).size(12.).color(HINT())).truncate());
            });
        });
    }

    /// Every reference: a pane beside the open one, or a card on its own when narrow.
    fn list(
        &self,
        ui: &mut Ui,
        card: bool,
        intents: &mut Vec<Intent>,
        widgets: &mut HashMap<String, UiControl>,
    ) {
        let mut rows = |ui: &mut Ui| {
            let scroll = ScrollArea::vertical()
                .id_salt("map-reference-sites")
                .auto_shrink([false, true])
                .show_rows(ui, ROW, self.sites.len(), |ui, rows| {
                    ui.spacing_mut().item_spacing.y = 0.;
                    for i in rows {
                        let on = !card && self.selected == Some(i);
                        if site_row(ui, widgets, &format!("map-reference-{i}"), &self.sites[i], on, card && i > 0) {
                            emit(intents, MapIntent::SelectReference(i));
                        }
                    }
                });
            pages::region(widgets, "map-reference-sites", scroll.inner_rect.intersect(ui.clip_rect()), true);
        };
        if card {
            Frame::new()
                .fill(PANEL())
                .stroke(Stroke::new(1., HAIRLINE()))
                .corner_radius(8)
                .inner_margin(Margin::symmetric(0, 6))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    rows(ui);
                });
        } else {
            Frame::new().inner_margin(Margin { left: 16, right: 14, top: 4, bottom: 6 }).show(ui, |ui| {
                details::section(ui, tr("References"), self.sites.len());
            });
            rows(ui);
        }
    }

    /// The open reference in `rect`: the notes Files shows above a file opened at a
    /// line, then the code in a framed card with the reference line banded and marked.
    fn source(&mut self, ui: &mut Ui, rect: Rect, widgets: &mut HashMap<String, UiControl>) {
        let Some(site) = self.selected.and_then(|i| self.sites.get(i)) else {
            return;
        };
        let inner = Rect::from_min_max(rect.min + vec2(16., 14.), rect.max - vec2(16., 16.));
        ui.scope_builder(UiBuilder::new().max_rect(inner), |ui| {
            ui.spacing_mut().item_spacing.y = 8.;
            if let Some(error) = &self.error {
                pages::callout(ui, if self.missing { AMBER() } else { RED() }, error);
                return;
            }
            let Some(content) = &self.content else {
                pages::loading(ui, tr("Opening the file"));
                return;
            };
            if !crate::tree::line_notes(ui, content, site.line as usize, false) {
                return;
            }
            let card = ui.available_rect_before_wrap();
            let gutter = crate::tree::gutter_width(content);
            let painter = ui.painter().clone();
            painter.rect_filled(card, 8., PANEL());
            let stripe = Rect::from_min_max(card.min, pos2(card.left() + 1. + gutter, card.bottom()));
            painter.rect_filled(stripe, CornerRadius { nw: 8, sw: 8, ne: 0, se: 0 }, BG());
            painter.vline(stripe.right() - 0.5, stripe.y_range(), Stroke::new(1., HAIRLINE()));
            let rows = Rect::from_min_max(card.min + vec2(1., 6.), card.max - vec2(1., 1.));
            let focus = (site.line as usize).checked_sub(content.first);
            let scroll = std::mem::take(&mut self.scroll).then_some(focus).flatten();
            ui.scope_builder(UiBuilder::new().max_rect(rows), |ui| {
                ui.set_clip_rect(rows.intersect(ui.clip_rect()));
                crate::tree::source_rows(
                    ui,
                    content,
                    focus,
                    scroll,
                    ("map-source", self.commit.as_str(), site.source.as_ref()),
                    "map-reference-source",
                    widgets,
                );
            });
            painter.rect_stroke(card, 8., Stroke::new(1., HAIRLINE()), StrokeKind::Inside);
            ui.advance_cursor_after_rect(card);
        });
    }
}

/// A square button with an arrow, as Files steps through its matches.
fn step_button(ui: &mut Ui, image: Icon, enabled: bool) -> Response {
    let r = ui.add_enabled(enabled, Button::new("").min_size(vec2(28., 28.)));
    let ink = if enabled { TEXT() } else { FAINT().gamma_multiply(0.6) };
    icon(ui.painter(), Rect::from_center_size(r.rect.center(), vec2(13., 13.)), image, ink);
    r
}

/// What kind of reference it is, as its reader names it: include_str!, stylesheet, @import…
fn kind_chip(ui: &mut Ui, kind: &str) {
    let galley = ui.painter().layout_no_wrap(kind.to_owned(), FontId::monospace(11.5), MUTED());
    let (rect, _) = ui.allocate_exact_size(vec2(galley.size().x + 12., 18.), Sense::hover());
    ui.painter().rect(rect, 4., RAISED(), Stroke::new(1., HAIRLINE()), StrokeKind::Inside);
    ui.painter().galley(pos2(rect.left() + 6., rect.center().y - galley.size().y / 2.), galley, MUTED());
}

/// A list row: the file's name, its folder and the line, then the kind and the file
/// it points to. The open one is banded with an accent bar; a card's rows are ruled.
fn site_row(
    ui: &mut Ui,
    widgets: &mut HashMap<String, UiControl>,
    id: &str,
    site: &AssetReference,
    on: bool,
    rule: bool,
) -> bool {
    let (rect, r) = ui.allocate_exact_size(vec2(ui.available_width(), ROW), Sense::click());
    let painter = ui.painter();
    if on {
        painter.rect_filled(rect, 0., SELECTED());
        painter.rect_filled(Rect::from_min_size(rect.min + vec2(0., 7.), vec2(3., ROW - 14.)), 2., BLUE());
    } else if r.hovered() {
        painter.rect_filled(rect, 0., HOVER());
    }
    if rule {
        painter.hline((rect.left() + 16.)..=(rect.right() - 14.), rect.top() + 0.5, Stroke::new(1., HAIRLINE()));
    }
    let (left, right) = (rect.left() + 16., rect.right() - 14.);
    let (top, bottom) = (rect.top() + 16., rect.top() + 36.);
    icon(painter, Rect::from_center_size(pos2(left + 7., top), vec2(14., 14.)), Icon::File, HINT());
    let line = painter.layout_no_wrap(
        trf!("Line {line}", line = pages::number_text(site.line as usize)),
        FontId::proportional(12.),
        HINT(),
    );
    let line_left = right - line.size().x;
    painter.galley(pos2(line_left, top - line.size().y / 2.), line, HINT());
    let source = path_label(&site.source);
    let (dir, name) = source.rsplit_once('/').unwrap_or(("", source.as_str()));
    let mut job = text::LayoutJob::default();
    job.append(name, 0., TextFormat::simple(semibold(13.), TEXT()));
    if !dir.is_empty() {
        job.append(&format!("  {dir}/"), 0., TextFormat::simple(FontId::proportional(12.), FAINT()));
    }
    job.wrap = text::TextWrapping::truncate_at_width((line_left - 10. - (left + 21.)).max(1.));
    let galley = painter.layout_job(job);
    painter.galley(pos2(left + 21., top - galley.size().y / 2.), galley, TEXT());
    let kind = painter.layout_no_wrap(site.kind.to_owned(), FontId::monospace(11.5), MUTED());
    let chip = Rect::from_min_size(pos2(left + 21., bottom - 9.), vec2(kind.size().x + 12., 18.));
    painter.rect(chip, 4., RAISED(), Stroke::new(1., HAIRLINE()), StrokeKind::Inside);
    painter.galley(pos2(chip.left() + 6., bottom - kind.size().y / 2.), kind, MUTED());
    let target = details::single(&format!("→  {}", path_label(&site.target)), FontId::proportional(12.), HINT(), right - chip.right() - 8.);
    let target = painter.layout_job(target);
    painter.galley(pos2(chip.right() + 8., bottom - target.size().y / 2.), target, HINT());
    let r = r.on_hover_text(format!("{source}:{} → {}", site.line, path_label(&site.target)));
    register(ui, widgets, id, &r);
    r.clicked()
}

/// Where the line names the file it points to, in characters: the file's name
/// (`"../docs/入門.md"` marks `入門.md`), else its module as a whole word (`mod model;`
/// or `use crate::model::Thing` for `src/model.rs` or `src/model/mod.rs`). None when the
/// line spells it some other way, such as percent-encoded; the line is still banded.
fn mark(line: &str, target: &[u8]) -> Option<Range<usize>> {
    let path = String::from_utf8_lossy(target);
    let mut parts = path.rsplit('/');
    let name = parts.next().filter(|n| !n.is_empty())?;
    if let Some(found) = whole(line, name) {
        return Some(found);
    }
    let stem = name.rsplit_once('.').map_or(name, |(stem, _)| stem);
    let stem = if stem == "mod" { parts.next().unwrap_or(stem) } else { stem };
    whole(line, stem)
}

/// The first place `word` stands on its own in `line`, as a character range.
fn whole(line: &str, word: &str) -> Option<Range<usize>> {
    let part = |c: char| c.is_alphanumeric() || matches!(c, '_' | '-' | '.');
    line.match_indices(word).find_map(|(at, _)| {
        let end = at + word.len();
        let before = line[..at].chars().next_back().is_none_or(|c| !part(c));
        let after = line[end..].chars().next().is_none_or(|c| !part(c) || c == '.' && line[end + 1..].chars().next().is_none_or(|c| !part(c)));
        (before && after).then(|| line[..at].chars().count()..line[..end].chars().count())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_mark_lands_on_the_name_the_line_uses() {
        let marked = |line: &str, target: &str| mark(line, target.as_bytes()).map(|r| line.chars().skip(r.start).take(r.len()).collect::<String>());
        // Embeds and web assets: the file's name inside its relative path.
        assert_eq!(marked(r#"    Guide { text: include_str!("../docs/入門.md") },"#, "docs/入門.md").as_deref(), Some("入門.md"));
        assert_eq!(marked(r#"        href="site.css">"#, "site.css").as_deref(), Some("site.css"));
        assert_eq!(marked(" background: url(&quot;雪.png&quot;)\"></div>", "雪.png").as_deref(), Some("雪.png"));
        // Not inside a longer name, and not the first look-alike when a real one follows.
        assert_eq!(mark(r#"<link href="data.css"><link href="a.css">"#, b"a.css"), Some(34..39));
        assert_eq!(marked("@import url(print/a.css.map);", "a.css"), None);
        // Rust modules: the module name as a word, a mod.rs by its folder.
        assert_eq!(marked("pub mod model;", "src/model.rs").as_deref(), Some("model"));
        assert_eq!(marked("use crate::model::Thing;", "src/model/mod.rs").as_deref(), Some("model"));
        assert_eq!(marked("use crate::models::Thing;", "src/model.rs"), None);
        // A sentence ending in the name still marks it; an encoded name marks nothing.
        assert_eq!(marked("// see site.css.", "site.css").as_deref(), Some("site.css"));
        assert_eq!(marked(r#"<img src="%E9%9B%AA.png">"#, "雪.png"), None);
    }

    fn site(source: &str, line: u32, target: &str, kind: &'static str) -> AssetReference {
        AssetReference { source: Arc::from(source.as_bytes()), line, target: Arc::from(target.as_bytes()), kind }
    }

    /// As `open` leaves a reference once its file has arrived: selected, and to be scrolled to.
    fn opened(view: &mut ReferenceView, index: usize, content: Content) {
        view.selected = Some(index);
        view.scroll = true;
        view.loaded(content);
    }

    fn text(lines: Vec<String>, first: usize, window: bool) -> Content {
        let kind = HistoricalFileKind::Text;
        Content { kind, bytes: 0, lines, limited: false, first, window, lossy: false, marks: HashMap::new() }
    }

    /// What one frame of the view registered, painted and copied, and whether it closed.
    struct Shown {
        controls: HashMap<String, UiControl>,
        painted: Vec<String>,
        copied: Vec<String>,
        intents: Vec<Intent>,
        closed: bool,
    }

    fn frame(ctx: &Context, view: &mut ReferenceView, size: Vec2, events: Vec<Event>) -> Shown {
        fn texts(shape: &Shape, out: &mut Vec<String>) {
            match shape {
                Shape::Text(text) => out.push(text.galley.text().to_owned()),
                Shape::Vec(shapes) => shapes.iter().for_each(|shape| texts(shape, out)),
                _ => {}
            }
        }
        let (mut controls, mut intents, mut closed) = (HashMap::new(), Vec::new(), false);
        let raw = RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)), events, ..Default::default() };
        let mut output = ctx.run_ui(raw, |ui| closed = view.show(ui, &mut intents, &mut controls));
        output.textures_delta.clear();
        let mut painted = Vec::new();
        output.shapes.iter().for_each(|clipped| texts(&clipped.shape, &mut painted));
        let copied = output.platform_output.commands.into_iter().filter_map(|c| match c {
            OutputCommand::CopyText(text) => Some(text),
            _ => None,
        });
        Shown { controls, painted, copied: copied.collect(), intents, closed }
    }

    fn settle(ctx: &Context, view: &mut ReferenceView, size: Vec2) -> Shown {
        frame(ctx, view, size, vec![]);
        frame(ctx, view, size, vec![])
    }

    fn click(ctx: &Context, view: &mut ReferenceView, size: Vec2, key: &str) -> Shown {
        let control = settle(ctx, view, size).controls.get(key).copied().unwrap_or_else(|| panic!("{key} is not shown"));
        assert!(control.enabled, "{key} is greyed out");
        let pos = control.rect.center();
        let press = |pressed| Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
        frame(ctx, view, size, vec![Event::PointerMoved(pos), press(true), press(false)])
    }

    fn escape() -> Event {
        Event::Key { key: Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }
    }

    #[test]
    fn the_viewer_says_what_is_open_and_steps_through_several_references() {
        let ctx = Context::default();
        crate::style::install(&ctx);
        let (wide, narrow) = (vec2(1480., 900.), vec2(780., 620.));
        let commit = ObjectId::parse(&format!("{:040x}", 0x197e_cbe1_u64)).unwrap();
        let sites = vec![
            site("頁.html", 7, "site.css", "stylesheet"),
            site("index.html", 9, "site.css", "stylesheet"),
            site("print/print.css", 1, "site.css", "@import"),
        ];
        let snapshot = "Snapshot of main · 197ecbe1".to_owned();
        let page = ["<head>", "  <link rel=\"stylesheet\"", "        href=\"site.css\">", "</head>"];
        // The stylesheet's href is line 7.
        let page: Vec<String> = ["<!doctype html>", "<html>", "", ""].iter().chain(&page).map(|l| l.to_string()).collect();
        let mut view = ReferenceView::new(commit.clone(), sites, b"site.css".to_vec(), snapshot.clone());
        opened(&mut view, 0, text(page, 1, false));
        assert_eq!(view.marked_text().as_deref(), Some("site.css"), "the referenced name is marked in its line");

        // Wide: the list beside the open reference, a title saying what is open, the
        // map's words for the snapshot, and the steps.
        let shown = settle(&ctx, &mut view, wide);
        for key in ["map-reference-close", "map-reference-0", "map-reference-1", "map-reference-2", "map-reference-copy", "map-reference-source-line-7"] {
            assert!(shown.controls.contains_key(key), "{key} is not shown");
        }
        assert!(!shown.controls.contains_key("map-reference-list"), "the list is already beside it");
        assert!(shown.controls["map-reference-next"].enabled && !shown.controls["map-reference-previous"].enabled);
        for words in ["Map", "頁.html  →  site.css", "stylesheet", "Line 7 · Snapshot of main · 197ecbe1", "1 of 3", "Copy location"] {
            assert!(shown.painted.iter().any(|t| t == words), "{words:?} is not painted: {:?}", shown.painted);
        }
        assert!(!shown.painted.iter().any(|t| t.contains(&commit.as_str()[..12])), "no raw 12-character hash");
        let next = click(&ctx, &mut view, wide, "map-reference-next");
        assert!(next.intents.iter().any(|i| matches!(i, Intent::Map(MapIntent::SelectReference(1)))));
        let row = click(&ctx, &mut view, wide, "map-reference-2");
        assert!(row.intents.iter().any(|i| matches!(i, Intent::Map(MapIntent::SelectReference(2)))));
        assert_eq!(click(&ctx, &mut view, wide, "map-reference-copy").copied, ["頁.html:7"]);
        assert!(settle(&ctx, &mut view, wide).painted.iter().any(|t| t == "Copied"));

        // Narrow: the open reference alone, back to the list first, then back to the map.
        let shown = settle(&ctx, &mut view, narrow);
        assert!(shown.controls.contains_key("map-reference-list") && !shown.controls.contains_key("map-reference-0"));
        assert!(shown.controls.contains_key("map-reference-next"), "the steps stay");
        assert!(!frame(&ctx, &mut view, narrow, vec![escape()]).closed);
        assert!(view.listing);
        let shown = settle(&ctx, &mut view, narrow);
        assert!(shown.controls.contains_key("map-reference-0") && shown.controls.contains_key("map-reference-close"));
        assert!(!shown.controls.contains_key("map-reference-copy") && !shown.controls.contains_key("map-reference-next"));
        assert!(shown.painted.iter().any(|t| t == "3 references · Snapshot of main · 197ecbe1"), "{:?}", shown.painted);
        assert!(frame(&ctx, &mut view, narrow, vec![escape()]).closed);
        // A wide window shows the list beside the open one whatever a narrow one last showed.
        assert!(settle(&ctx, &mut view, wide).controls.contains_key("map-reference-source-line-7"));
        assert!(frame(&ctx, &mut view, wide, vec![escape()]).closed, "Esc goes back to the map");

        // One reference in a large file: no list or steps, and the window is said.
        let lines: Vec<String> = (55_526..=55_926).map(|n| if n == 55_726 { "pub const GUIDE: &str = include_str!(\"../入門.md\");".into() } else { String::new() }).collect();
        let mut view = ReferenceView::new(commit.clone(), vec![site("src/lib.rs", 55_726, "入門.md", "include_str!")], b"src/lib.rs".to_vec(), snapshot.clone());
        opened(&mut view, 0, text(lines, 55_526, true));
        assert_eq!(view.marked_text().as_deref(), Some("入門.md"));
        let shown = settle(&ctx, &mut view, wide);
        assert!(!shown.controls.contains_key("map-reference-next") && !shown.controls.contains_key("map-reference-0"));
        assert!(shown.controls.contains_key("map-reference-source-line-55726"));
        assert!(shown.painted.iter().any(|t| t == "Large file: showing lines 55,526 to 55,926, around line 55,726"), "{:?}", shown.painted);
        assert!(shown.painted.iter().any(|t| t == "Line 55,726 · Snapshot of main · 197ecbe1"));

        // A line the commit doesn't have: today's words, and no code.
        let mut view = ReferenceView::new(commit, vec![site("src/guide.rs", 42, "docs/入門.md", "include_str!")], b"src/guide.rs".to_vec(), snapshot);
        opened(&mut view, 0, text(vec!["fn main() {}".into()], 1, false));
        let shown = settle(&ctx, &mut view, narrow);
        assert!(shown.painted.iter().any(|t| t == "This reference line is unavailable in the selected commit."));
        assert!(!shown.controls.contains_key("map-reference-source"));
    }
}
