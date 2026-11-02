//! Lazy, bounded image comparison. Only decoded pixels cross back to the UI.
use crate::{app::UiControl, backend::user_error, diff::FilePatch, style::*};
use eframe::egui::*;
use image::ImageDecoder;
use rocket_git::{
    CancellationToken, DiagnosticLog, Error, ImageComparison, ReadTask, Repository, Result,
};
use std::{
    collections::HashMap,
    io::{self, BufRead, Cursor, Read, Seek, SeekFrom},
    sync::{Arc, Mutex, TryLockError},
    time::Duration,
};

const MAX_SIDE: u32 = 4096;
const MAX_PIXELS: u64 = 8 * 1024 * 1024;
static DECODER: Mutex<()> = Mutex::new(());
mod composite;
mod cache;
mod pixels;
pub use pixels::Stats;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    Side,
    Overlay,
    Swipe,
    Pixels,
}
impl Mode {
    pub fn code(self) -> &'static str {
        match self {
            Self::Side => "side",
            Self::Overlay => "overlay",
            Self::Swipe => "swipe",
            Self::Pixels => "pixels",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Info {
    pub width: usize,
    pub height: usize,
    pub encoded_bytes: usize,
    pub format: &'static str,
    pub alpha: bool,
}
struct Decoded {
    info: Info,
    pixels: ColorImage,
}
struct Raw {
    info: Info,
    rgba: Vec<u8>,
}
impl Raw {
    fn display(self, cancel: &CancellationToken) -> Result<Decoded> {
        cancel.check()?;
        let pixels =
            ColorImage::from_rgba_unmultiplied([self.info.width, self.info.height], &self.rgba);
        cancel.check()?;
        Ok(Decoded {
            info: self.info,
            pixels,
        })
    }
}
struct Pair {
    before: Option<Decoded>,
    after: Option<Decoded>,
    difference: Option<pixels::Difference>,
    difference_error: Option<String>,
}

pub struct State {
    pub before: Option<Info>,
    pub after: Option<Info>,
    pub loaded: bool,
    pub error: Option<String>,
    pub cancelled: bool,
    pub fit: bool,
    pub zoom: f32,
    pub checker: bool,
    pub mode: Mode,
    pub mix: f32,
    pub stats: Option<Stats>,
    pub difference_error: Option<String>,
    pub log: Option<DiagnosticLog>,
    key: u64,
    task: Option<ReadTask<Pair>>,
    resident: Arc<Mutex<cache::Cached>>,
}
impl Default for State {
    fn default() -> Self {
        Self {
            before: None,
            after: None,
            loaded: false,
            error: None,
            cancelled: false,
            fit: true,
            zoom: 1.0,
            checker: true,
            mode: Mode::Side,
            mix: 0.5,
            stats: None,
            difference_error: None,
            log: None,
            key: 0,
            task: None,
            resident: Default::default(),
        }
    }
}
impl State {
    fn textures(&self) -> [Option<TextureHandle>; 3] {
        self.resident.lock().unwrap().handles.clone()
    }
    pub fn with_log(log: Option<DiagnosticLog>) -> Self {
        Self {
            log,
            ..Default::default()
        }
    }
    pub fn running(&self) -> bool {
        self.task.is_some()
    }
    pub fn clear(&mut self) {
        *self = Self {
            log: self.log.clone(),
            fit: self.fit,
            zoom: self.zoom,
            checker: self.checker,
            mode: self.mode,
            mix: self.mix,
            ..Default::default()
        };
    }
    pub fn stop(&mut self) {
        if self.task.take().is_some() {
            self.cancelled = true;
            self.error = Some("Image preview cancelled. Choose Reload to try again.".into());
        }
    }
    pub fn poll(&mut self, ctx: &Context) {
        self.observe_eviction();
        if let Some(result) = self.task.as_mut().and_then(ReadTask::try_finish) {
            self.task = None;
            match result {
                Ok(pair) => {
                    let gpu_limit = ctx.input(|i| i.max_texture_side);
                    if pair
                        .before
                        .iter()
                        .chain(pair.after.iter())
                        .any(|d| d.info.width > gpu_limit || d.info.height > gpu_limit)
                    {
                        self.error=Some("Image exceeds this graphics device's texture size limit. Use an external viewer.".into());
                        return;
                    }
                    let bytes = pair.before.iter().chain(pair.after.iter())
                        .map(|d| d.pixels.pixels.len() * 4).sum::<usize>()
                        + pair.difference.as_ref().map_or(0, |d| d.image.pixels.len() * 4);
                    if !cache::admit(ctx, &self.resident, bytes) {
                        self.error = Some("Image preview exceeds the shared memory budget. Use an external viewer.".into());
                        return;
                    }
                    let mut resident = self.resident.lock().unwrap();
                    for (i, decoded) in [pair.before, pair.after].into_iter().enumerate() {
                        if let Some(decoded) = decoded {
                            resident.handles[i] = Some(ctx.load_texture(
                                format!("diff-image-{}-{i}", self.key),
                                decoded.pixels,
                                TextureOptions::LINEAR,
                            ));
                            if i == 0 {
                                self.before = Some(decoded.info);
                            } else {
                                self.after = Some(decoded.info);
                            }
                        }
                    }
                    if let Some(d) = pair.difference {
                        self.stats = Some(d.stats);
                        resident.handles[2] = Some(ctx.load_texture(
                            format!("diff-mask-{}", self.key),
                            d.image,
                            TextureOptions::NEAREST,
                        ));
                    }
                    self.difference_error = pair.difference_error;
                    self.loaded = true;
                }
                Err(e) => self.error = Some(user_error(&e)),
            }
        }
    }
    fn observe_eviction(&mut self) {
        if self.loaded && self.resident.lock().unwrap().handles.iter().all(Option::is_none) {
            self.loaded = false;
            self.stats = None;
            self.error = Some("Preview released to keep image memory bounded. Choose Reload to view it again.".into());
        }
    }
    fn start(&mut self, key: u64, selection: ImageComparison, repository: Repository) {
        self.clear();
        self.key = key;
        let log = self.log.clone();
        let differences = self.mode == Mode::Pixels;
        match ReadTask::spawn(move |cancel| {
            if let Some(log) = log {
                log.request("image-preview", |log| {
                    load(
                        &repository.with_log(log.clone()),
                        &selection,
                        Some(log),
                        differences,
                        &cancel,
                    )
                })
            } else {
                load(&repository, &selection, None, differences, &cancel)
            }
        }) {
            Ok(task) => self.task = Some(task),
            Err(e) => self.error = Some(user_error(&e)),
        }
    }
    /// Returns true when fresh Git identities (not just the old blobs) are requested.
    pub(crate) fn ui(
        &mut self,
        ui: &mut Ui,
        file: &FilePatch,
        repository: &Repository,
        widgets: &mut HashMap<String, UiControl>,
    ) -> bool {
        let Some(selection) = file.image.as_ref() else {
            return false;
        };
        if self.key != file.id {
            self.start(file.id, selection.clone(), repository.clone());
        }
        self.observe_eviction();
        if self.loaded { self.resident.lock().unwrap().touch(); }
        let compact = ui.available_width() < 440.0 || ui.available_height() < 300.0;
        if compact {
            ui.spacing_mut().interact_size.y = 22.0;
            ui.spacing_mut().button_padding = vec2(6.0, 2.0);
            ui.spacing_mut().item_spacing = vec2(4.0, 3.0);
        }
        let mut reload = false;
        ui.horizontal_wrapped(|ui| {
            for (id, label) in [
                ("image-fit", "Fit"),
                ("image-actual", "100%"),
                ("image-zoom-out", "−"),
                ("image-zoom-in", "+"),
            ] {
                let r = ui.add_enabled(
                    self.loaded,
                    Button::new(label).selected(
                        (id == "image-fit" && self.fit)
                            || (id == "image-actual" && !self.fit && self.zoom == 1.0),
                    ),
                );
                widgets.insert(id.into(), (&r).into());
                if r.clicked() {
                    match id {
                        "image-fit" => self.fit = true,
                        "image-actual" => {
                            self.fit = false;
                            self.zoom = 1.0
                        }
                        "image-zoom-out" => {
                            self.fit = false;
                            self.zoom = (self.zoom / 1.25).max(0.125)
                        }
                        _ => {
                            self.fit = false;
                            self.zoom = (self.zoom * 1.25).min(4.0)
                        }
                    }
                }
            }
            if !compact || !self.fit {
                ui.label(if self.fit {
                    "Fit".into()
                } else {
                    format!("{:.0}%", self.zoom * 100.0)
                });
            }
            let r = ui
                .add(checkbox(
                    &mut self.checker,
                    if compact { "Grid" } else { "Transparency grid" },
                ))
                .on_hover_text("Show a checkerboard behind transparent pixels");
            widgets.insert("image-checker".into(), (&r).into());
            let r = ui.add_enabled(!self.running(), Button::new("Reload"));
            widgets.insert("image-reload".into(), (&r).into());
            reload = r.clicked();
            if self.running() {
                let r = ui.button("Stop");
                widgets.insert("image-cancel".into(), (&r).into());
                if r.clicked() {
                    self.stop();
                }
            }
        });
        if self.running() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Loading image comparison…");
            });
            ui.ctx().request_repaint_after(Duration::from_millis(16));
        } else if let Some(error) = &self.error {
            ui.colored_label(AMBER(), error);
        } else if self.loaded {
            ui.horizontal_wrapped(|ui| {
                for (mode, label) in [
                    (Mode::Side, "Side by side"),
                    (Mode::Overlay, "Overlay"),
                    (Mode::Swipe, "Swipe"),
                    (Mode::Pixels, "Pixel differences"),
                ] {
                    let r = ui.selectable_label(self.mode == mode, label);
                    widgets.insert(format!("image-mode-{}", mode.code()), (&r).into());
                    if r.clicked() {
                        self.mode = mode;
                    }
                }
            });
            if self.mode == Mode::Pixels && self.textures()[2].is_none() && self.difference_error.is_none() {
                // Re-decode the reviewed identities on a worker rather than keep
                // an extra full CPU pixel copy for every open comparison.
                self.start(file.id, selection.clone(), repository.clone());
                ui.spinner();
                ui.ctx().request_repaint();
                return reload;
            }
            if self.mode != Mode::Pixels {
                self.resident.lock().unwrap().handles[2] = None;
            }
            if self.mode != Mode::Side {
                self.composite(ui, selection, widgets);
                return reload;
            }
            let stack = ui.available_width() < 260.0;
            let panels = if stack { 2.0 } else { 1.0 };
            let height = ((ui.available_height()
                - if compact {
                    panels * 43.0 + 4.0
                } else {
                    panels * 52.0 + 32.0
                })
                / panels)
                .max(24.0);
            let width = if stack {
                ui.available_width()
            } else {
                (ui.available_width() - 12.0) / 2.0
            };
            let max_w = self
                .before
                .iter()
                .chain(self.after.iter())
                .map(|i| i.width)
                .max()
                .unwrap_or(1) as f32;
            let max_h = self
                .before
                .iter()
                .chain(self.after.iter())
                .map(|i| i.height)
                .max()
                .unwrap_or(1) as f32;
            let scale = if self.fit {
                (width / max_w).min(height / max_h).min(1.0)
            } else {
                self.zoom
            };
            let labels = [(0, selection.before_label()), (1, selection.after_label())];
            let scroll = ScrollArea::vertical()
                .id_salt(("image-pair", file.id))
                .min_scrolled_height(0.0)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    if stack {
                        for (i, label) in labels {
                            self.pane(ui, i, label, height, scale, compact, widgets);
                        }
                    } else {
                        ui.columns(2, |columns| {
                            for (i, label) in labels {
                                columns[i].with_layout(Layout::top_down(Align::Min), |ui| {
                                    self.pane(ui, i, label, height, scale, compact, widgets);
                                });
                            }
                        });
                    }
                    if !compact {
                        ui.small(concat!(
                            "Read-only preview · PNG, JPEG, BMP, TGA, WebP · ",
                            "Whole-file staging uses the original bytes."
                        ));
                    }
                });
            let rect = scroll.inner_rect.intersect(ui.clip_rect());
            if rect.is_positive() {
                widgets.insert(
                    "image-scroll".into(),
                    UiControl {
                        rect,
                        enabled: true,
                    },
                );
            }
        }
        reload
    }
    #[allow(clippy::too_many_arguments)]
    fn pane(
        &self,
        ui: &mut Ui,
        side: usize,
        source: &str,
        height: f32,
        scale: f32,
        compact: bool,
        widgets: &mut HashMap<String, UiControl>,
    ) {
        let info = if side == 0 {
            self.before.as_ref()
        } else {
            self.after.as_ref()
        };
        ui.add(
            Label::new(
                RichText::new(format!(
                    "{} · {source}",
                    if side == 0 { "Before" } else { "After" }
                ))
                .color(MUTED()),
            )
            .truncate(),
        )
        .on_hover_text(source);
        if let Some(info) = info {
            let details = format!(
                "{} · {} × {} · {:.1} KiB{}",
                info.format,
                info.width,
                info.height,
                info.encoded_bytes as f32 / 1024.0,
                if info.alpha { " · alpha" } else { "" }
            );
            ui.add(
                Label::new(
                    RichText::new(if compact {
                        format!("{} × {}", info.width, info.height)
                    } else {
                        details.clone()
                    })
                    .small(),
                )
                .truncate(),
            )
            .on_hover_text(details);
        } else {
            ui.small("Not present in this version");
        }
        let scroll = ScrollArea::both()
            .id_salt(("image-side", self.key, side))
            .max_height(height)
            .min_scrolled_height(0.0)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let size = info
                    .map(|i| vec2(i.width as f32 * scale, i.height as f32 * scale))
                    .unwrap_or(vec2(1.0, 1.0));
                let canvas = vec2(ui.available_width().max(size.x), height.max(size.y));
                let (rect, _) = ui.allocate_exact_size(canvas, Sense::hover());
                let image_rect = Rect::from_center_size(rect.center(), size);
                let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
                painter.rect_filled(rect, 0, IMAGE_BG());
                let textures = self.textures();
                if textures[side].is_some() && self.checker {
                    checkerboard(&painter, image_rect);
                }
                if let Some(texture) = &textures[side] {
                    painter.image(
                        texture.id(),
                        image_rect,
                        Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)),
                        Color32::WHITE,
                    );
                } else {
                    painter.text(
                        rect.center(),
                        Align2::CENTER_CENTER,
                        "Not present",
                        FontId::proportional(14.0),
                        MUTED(),
                    );
                }
            });
        let rect = scroll.inner_rect.intersect(ui.clip_rect());
        if rect.is_positive() {
            widgets.insert(
                format!("image-side-{side}"),
                UiControl {
                    rect,
                    enabled: true,
                },
            );
        }
    }
}
fn checkerboard(painter: &Painter, rect: Rect) {
    let visible = rect.intersect(painter.clip_rect());
    if !visible.is_positive() {
        return;
    }
    painter.rect_filled(visible, 0, CHECKER_A());
    let x0 = ((visible.left() - rect.left()) / 16.0).floor() as i32;
    let y0 = ((visible.top() - rect.top()) / 16.0).floor() as i32;
    let x1 = ((visible.right() - rect.left()) / 16.0).ceil() as i32;
    let y1 = ((visible.bottom() - rect.top()) / 16.0).ceil() as i32;
    for y in y0..y1 {
        for x in x0..x1 {
            if (x + y) % 2 == 0 {
                painter.rect_filled(
                    Rect::from_min_size(
                        rect.min + vec2(x as f32 * 16.0, y as f32 * 16.0),
                        vec2(16.0, 16.0),
                    )
                    .intersect(visible),
                    0,
                    CHECKER_B(),
                );
            }
        }
    }
}
fn load(
    repository: &Repository,
    selection: &ImageComparison,
    log: Option<&DiagnosticLog>,
    differences: bool,
    cancel: &CancellationToken,
) -> Result<Pair> {
    // At most one active image decode, even during rapid file/tab changes. Waiters
    // have no image buffers and observe cancellation before taking this permit.
    let _permit = loop {
        cancel.check()?;
        match DECODER.try_lock() {
            Ok(p) => break p,
            Err(TryLockError::WouldBlock) => std::thread::sleep(Duration::from_millis(10)),
            Err(TryLockError::Poisoned(_)) => return Err(Error::StatePoisoned("image decoder")),
        }
    };
    let versions = repository.image_versions(selection, cancel)?;
    let tga = selection
        .path()
        .as_bytes()
        .rsplit(|b| *b == b'.')
        .next()
        .is_some_and(|ext| ext.eq_ignore_ascii_case(b"tga"));
    let decode_side = |bytes: Option<Vec<u8>>, label: &'static str| -> Result<Option<Raw>> {
        match bytes {
            Some(bytes) => {
                if let Some(log) = log {
                    log.request(label, |_| decode(bytes, tga, cancel)).map(Some)
                } else {
                    decode(bytes, tga, cancel).map(Some)
                }
            }
            None => Ok(None),
        }
    };
    let before = decode_side(versions.before, "image-decode-before")?;
    let after = decode_side(versions.after, "image-decode-after")?;
    cancel.check()?;
    let difference = if !differences {
        None
    } else if let Some(log) = log {
        Some(log.request("image-difference", |_| {
            pixels::compare(before.as_ref(), after.as_ref(), cancel)
        }))
    } else {
        Some(pixels::compare(before.as_ref(), after.as_ref(), cancel))
    };
    let (difference, difference_error) = match difference {
        None => (None, None),
        Some(Ok(d)) => (Some(d), None),
        Some(Err(Error::Unsupported(message))) => (None, Some(message)),
        Some(Err(e)) => return Err(e),
    };
    Ok(Pair {
        before: before.map(|r| r.display(cancel)).transpose()?,
        after: after.map(|r| r.display(cancel)).transpose()?,
        difference,
        difference_error,
    })
}
/// `tga`: the file is named `.tga`. TGA has no signature, so only such a name makes
/// bytes without a recognized signature decode as TGA.
fn decode(bytes: Vec<u8>, tga: bool, cancel: &CancellationToken) -> Result<Raw> {
    cancel.check()?;
    if bytes.len() > rocket_git::IMAGE_FILE_LIMIT {
        return Err(Error::Unsupported(
            "Image exceeds the 16 MiB file preview limit.".into(),
        ));
    }
    let (format, name) = match image::guess_format(&bytes) {
        Ok(image::ImageFormat::Png) => (image::ImageFormat::Png, "PNG"),
        Ok(image::ImageFormat::Jpeg) => (image::ImageFormat::Jpeg, "JPEG"),
        Ok(image::ImageFormat::Bmp) => (image::ImageFormat::Bmp, "BMP"),
        Ok(image::ImageFormat::WebP) => (image::ImageFormat::WebP, "WebP"),
        _ if tga => (image::ImageFormat::Tga, "TGA"),
        Ok(_) => return Err(Error::Unsupported(
            "This format is not supported by the image viewer. PNG, JPEG, BMP, TGA and WebP are supported."
                .into(),
        )),
        Err(_) => return Err(invalid_image()),
    };
    let encoded_bytes = bytes.len();
    let reader = CheckedReader {
        cursor: Cursor::new(bytes),
        cancel: cancel.clone(),
    };
    let mut reader = image::ImageReader::with_format(reader, format);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_SIDE);
    limits.max_image_height = Some(MAX_SIDE);
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits);
    let decode_error = |e| {
        if cancel.check().is_err() {
            Error::Cancelled
        } else {
            match e {
                image::ImageError::Limits(_) => large_image(),
                _ => invalid_image(),
            }
        }
    };
    let mut decoder = reader.into_decoder().map_err(decode_error)?;
    let (width, height) = decoder.dimensions();
    if width == 0
        || height == 0
        || width > MAX_SIDE
        || height > MAX_SIDE
        || u64::from(width) * u64::from(height) > MAX_PIXELS
        || decoder.total_bytes() > 64 * 1024 * 1024
    {
        return Err(large_image());
    }
    let orientation = decoder.orientation().map_err(decode_error)?;
    cancel.check()?;
    let mut decoded = image::DynamicImage::from_decoder(decoder).map_err(decode_error)?;
    cancel.check()?;
    decoded.apply_orientation(orientation);
    let rgba = decoded.into_rgba8();
    let size = [rgba.width() as usize, rgba.height() as usize];
    let alpha = rgba.pixels().any(|pixel| pixel[3] != 255);
    cancel.check()?;
    Ok(Raw {
        info: Info {
            width: size[0],
            height: size[1],
            encoded_bytes,
            format: name,
            alpha,
        },
        rgba: rgba.into_raw(),
    })
}
fn invalid_image() -> Error {
    Error::InvalidInput("This image is damaged or could not be decoded. Original bytes are unchanged; check it in another viewer, then reload.".into())
}
fn large_image() -> Error {
    Error::Unsupported("Image exceeds the preview limit (4096 pixels per side, 8 megapixels, bounded decoder memory). Whole-file staging remains available.".into())
}
struct CheckedReader {
    cursor: Cursor<Vec<u8>>,
    cancel: CancellationToken,
}
impl CheckedReader {
    fn check(&self) -> io::Result<()> {
        self.cancel
            .check()
            .map_err(|_| io::Error::other("image preview cancelled"))
    }
}
impl Read for CheckedReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.check()?;
        let end = buf.len().min(16384);
        self.cursor.read(&mut buf[..end])
    }
}
impl BufRead for CheckedReader {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        self.check()?;
        let bytes = self.cursor.fill_buf()?;
        Ok(&bytes[..bytes.len().min(16384)])
    }
    fn consume(&mut self, n: usize) {
        self.cursor.consume(n)
    }
}
impl Seek for CheckedReader {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.check()?;
        self.cursor.seek(pos)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(windows)]
    #[test]
    fn native_mode_controls_load_difference_lazily_and_release_textures() {
        crate::test_common::isolated("ROCKET_IMAGE_LAZY_CASE",
            "image_view::tests::native_mode_controls_load_difference_lazily_and_release_textures", || {
            let f = crate::test_common::Fixture::new();
            f.write("comparison.png", encoded(image::ImageFormat::Png, 32, 24));
            f.commit("before image");
            f.write("comparison.png", encoded(image::ImageFormat::Png, 40, 24));
            let repo = f.repo();
            let options = rocket_git::DiffOptions { target: rocket_git::DiffTarget::Unstaged, ..Default::default() };
            let comparison = repo.image_comparisons(&options, &CancellationToken::new()).unwrap().remove(0);
            let file = FilePatch { id: 1, image: Some(comparison), ..Default::default() };
            let ctx = Context::default();
            crate::style::install(&ctx);
            let mut state = State::default();
            let mut controls = HashMap::new();
            fn frame(ctx: &Context, state: &mut State, repo: &Repository, file: &FilePatch,
                controls: &mut HashMap<String, UiControl>, events: Vec<Event>, size: Vec2) -> bool {
                controls.clear();
                let mut reload = false;
                let mut output = ctx.run_ui(RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)), events, ..Default::default()
                }, |ui| { reload = state.ui(ui, file, repo, controls); });
                output.textures_delta.clear(); reload
            }
            fn settle(ctx: &Context, state: &mut State) {
                let started = std::time::Instant::now();
                while state.running() {
                    state.poll(ctx);
                    assert!(started.elapsed() < Duration::from_secs(15));
                    std::thread::sleep(Duration::from_millis(5));
                }
                assert!(state.loaded, "{:?}", state.error);
            }
            let size = vec2(780.0, 620.0);
            frame(&ctx, &mut state, &repo, &file, &mut controls, vec![], size);
            settle(&ctx, &mut state);
            assert!(state.textures()[0].is_some() && state.textures()[1].is_some());
            assert!(state.textures()[2].is_none() && state.stats.is_none());
            for (key, pixels) in [("image-mode-pixels", true), ("image-mode-side", false)] {
                frame(&ctx, &mut state, &repo, &file, &mut controls, vec![], size);
                let pos = controls[key].rect.center();
                for pressed in [true, false] {
                    frame(&ctx, &mut state, &repo, &file, &mut controls, vec![Event::PointerMoved(pos),
                        Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE }], size);
                }
                settle(&ctx, &mut state);
                assert_eq!(state.textures()[2].is_some(), pixels);
                if pixels { assert_eq!(state.stats.as_ref().unwrap().changed, 8 * 24); }
            }
            frame(&ctx, &mut state, &repo, &file, &mut controls, vec![], vec2(1280.0, 860.0));
            let ids: Vec<_> = state.textures().into_iter().flatten().map(|t| t.id()).collect();
            state.resident.lock().unwrap().handles = Default::default();
            state.poll(&ctx);
            assert!(!state.loaded && state.error.as_deref().unwrap().contains("Reload"));
            assert!(ids.into_iter().all(|id| ctx.tex_manager().read().meta(id).is_none()));
            frame(&ctx, &mut state, &repo, &file, &mut controls, vec![], size);
            assert!(!state.running(), "eviction must not start a decode loop across windows");
            state.clear();
            assert!(state.textures().iter().all(Option::is_none));
        });
    }
    fn encoded(format: image::ImageFormat, width: u32, height: u32) -> Vec<u8> {
        let image = image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            width,
            height,
            image::Rgba([45, 125, 220, 128]),
        ));
        let mut out = Cursor::new(Vec::new());
        if format == image::ImageFormat::Jpeg {
            image.to_rgb8().write_to(&mut out, format).unwrap()
        } else {
            image.write_to(&mut out, format).unwrap()
        }
        out.into_inner()
    }
    #[test]
    fn formats_transparency_limits_and_cancellation() {
        for (format, name) in [
            (image::ImageFormat::Png, "PNG"),
            (image::ImageFormat::Jpeg, "JPEG"),
            (image::ImageFormat::Bmp, "BMP"),
            (image::ImageFormat::Tga, "TGA"),
            // Lossless, with the half-transparent pixels kept.
            (image::ImageFormat::WebP, "WebP"),
        ] {
            let tga = format == image::ImageFormat::Tga;
            let decoded = decode(encoded(format, 32, 24), tga, &CancellationToken::new()).unwrap();
            assert_eq!((decoded.info.width, decoded.info.height), (32, 24));
            assert_eq!(decoded.info.alpha, format != image::ImageFormat::Jpeg);
            assert_eq!(decoded.info.format, name);
        }
        // WebP is known by its signature, also under a .tga name; a truncated one fails cleanly.
        let webp = encoded(image::ImageFormat::WebP, 8, 8);
        let named_tga = decode(webp.clone(), true, &CancellationToken::new()).unwrap();
        assert_eq!(named_tga.info.format, "WebP");
        assert!(matches!(
            decode(
                webp[..webp.len() / 2].to_vec(),
                false,
                &CancellationToken::new()
            ),
            Err(Error::InvalidInput(_))
        ));
        // TGA has no signature: without a .tga name it is not an image; a .tga name
        // does not override the signature of another format; damaged TGA fails cleanly.
        let tga = encoded(image::ImageFormat::Tga, 8, 8);
        assert!(decode(tga.clone(), false, &CancellationToken::new()).is_err());
        let png = decode(
            encoded(image::ImageFormat::Png, 8, 8),
            true,
            &CancellationToken::new(),
        );
        assert_eq!(png.unwrap().info.format, "PNG");
        assert!(matches!(
            decode(tga[..20].to_vec(), true, &CancellationToken::new()),
            Err(Error::InvalidInput(_))
        ));
        assert!(decode(b"not an image".to_vec(), false, &CancellationToken::new()).is_err());
        assert!(matches!(
            decode(
                encoded(image::ImageFormat::Png, 4097, 1),
                false,
                &CancellationToken::new()
            ),
            Err(Error::Unsupported(_))
        ));
        let cancel = CancellationToken::new();
        cancel.cancel();
        assert!(matches!(
            decode(Vec::new(), false, &cancel),
            Err(Error::Cancelled)
        ));
        let mut reader = CheckedReader {
            cursor: Cursor::new(vec![0; 32]),
            cancel,
        };
        assert!(reader.read(&mut [0; 8]).is_err());
    }
}
