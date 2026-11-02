//! The Custom theme's picture: one of Rocket's own or the user's, copied into Rocket's
//! storage. It is decoded and prepared off the UI thread, dimmed (or, on Light,
//! lifted) where text over the see-through panes would lose contrast, and painted
//! behind the main window's panes.
use super::{Base, Custom, Picture, Position, Size, linear, luminance};
use eframe::egui::*;
use image::ImageDecoder;
use std::{
    collections::HashMap,
    hash::{Hash, Hasher},
    io::Read,
    path::{Path, PathBuf},
    sync::{Arc, OnceLock, mpsc},
};

/// One of Rocket's pictures. The SVG sources are in assets/themes/src.
struct BuiltIn {
    /// The picture's [`Picture::key`].
    key: &'static str,
    /// 1920 × 1200 JPEG.
    picture: &'static [u8],
    /// Its Preferences tile.
    tile: &'static [u8],
    /// The colour it was drawn around, which From picture uses.
    accent: [u8; 3],
}
const BUILT_IN: [BuiltIn; 4] = [
    BuiltIn {
        key: "astronaut",
        picture: include_bytes!("../../assets/themes/astronaut.jpg"),
        tile: include_bytes!("../../assets/themes/astronaut-tile.jpg"),
        accent: [244, 114, 182],
    },
    BuiltIn {
        key: "space-cat",
        picture: include_bytes!("../../assets/themes/space-cat.jpg"),
        tile: include_bytes!("../../assets/themes/space-cat-tile.jpg"),
        accent: [242, 179, 91],
    },
    BuiltIn {
        key: "planet",
        picture: include_bytes!("../../assets/themes/planet.jpg"),
        tile: include_bytes!("../../assets/themes/planet-tile.jpg"),
        accent: [167, 139, 250],
    },
    BuiltIn {
        key: "stars",
        picture: include_bytes!("../../assets/themes/stars.jpg"),
        tile: include_bytes!("../../assets/themes/stars-tile.jpg"),
        accent: [34, 195, 238],
    },
];

fn built_in(picture: &Picture) -> Option<&'static BuiltIn> {
    BUILT_IN.iter().find(|b| b.key == picture.key())
}
/// A chosen picture file larger than this is refused.
pub const FILE_LIMIT: u64 = 32 * 1024 * 1024;
/// Pictures with a longer side or more pixels are refused rather than decoded.
const DECODE_SIDE: u32 = 12_000;
const DECODE_PIXELS: u64 = 48 * 1024 * 1024;
/// Pictures are kept and shown at most this many pixels on their long side.
pub const KEPT_SIDE: u32 = 2560;

/// Where chosen pictures are kept, inside Rocket's state folder.
pub fn folder(storage: &Path) -> PathBuf {
    storage.join("themes")
}

/// Names Rocket gives the pictures it keeps: `picture-` and 16 lowercase hex digits.
pub(crate) fn own_name(name: &str) -> bool {
    name.strip_prefix("picture-")
        .and_then(|rest| rest.strip_suffix(".png"))
        .is_some_and(|hex| hex.len() == 16 && hex.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)))
}

/// Copies the picture at `source` into Rocket's storage, scaled down to KEPT_SIDE,
/// and removes the picture chosen before it. Returns the kept picture's name.
pub fn import(source: &Path, storage: &Path) -> Result<String, String> {
    let image = decode(&read(source)?)?;
    let folder = folder(storage);
    std::fs::create_dir_all(&folder).map_err(|e| format!("Rocket could not keep the picture: {e}."))?;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    (image.width(), image.height(), image.as_raw()).hash(&mut hasher);
    let name = format!("picture-{:016x}.png", hasher.finish());
    let temporary = folder.join(format!("{name}.{}.tmp", std::process::id()));
    let written = image
        .save_with_format(&temporary, image::ImageFormat::Png)
        .map_err(|e| e.to_string())
        .and_then(|()| std::fs::rename(&temporary, folder.join(&name)).map_err(|e| e.to_string()));
    if let Err(e) = written {
        let _ = std::fs::remove_file(&temporary);
        return Err(format!("Rocket could not keep the picture: {e}."));
    }
    if let Ok(entries) = std::fs::read_dir(&folder) {
        for entry in entries.flatten() {
            let old = entry.file_name();
            // The picture chosen before, and copies a closed Rocket left half written.
            let stale = |old: &str| {
                (own_name(old) && old != name) || (old.starts_with("picture-") && old.ends_with(".tmp"))
            };
            if old.to_str().is_some_and(stale) {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
    Ok(name)
}

fn read(path: &Path) -> Result<Vec<u8>, String> {
    let file = std::fs::File::open(path).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => "The picture is missing. Choose it again.".to_owned(),
        _ => format!("The picture could not be read: {e}."),
    })?;
    let mut bytes = Vec::new();
    file.take(FILE_LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("The picture could not be read: {e}."))?;
    if bytes.len() as u64 > FILE_LIMIT {
        return Err("The picture is larger than 32 MB.".into());
    }
    Ok(bytes)
}

/// A PNG, JPEG or WebP picture within the limits, upright and at most KEPT_SIDE.
fn decode(bytes: &[u8]) -> Result<image::RgbaImage, String> {
    let format = match image::guess_format(bytes) {
        Ok(format @ (image::ImageFormat::Png | image::ImageFormat::Jpeg | image::ImageFormat::WebP)) => format,
        _ => return Err("Choose a PNG, JPG or WebP picture.".into()),
    };
    let too_large = || format!("The picture is too large; Rocket reads pictures up to {DECODE_SIDE} pixels on a side.");
    let unreadable = || "The picture could not be read.".to_owned();
    let failed = |e: image::ImageError| match e {
        image::ImageError::Limits(_) => too_large(),
        _ => unreadable(),
    };
    let mut reader = image::ImageReader::with_format(std::io::Cursor::new(bytes), format);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(DECODE_SIDE);
    limits.max_image_height = Some(DECODE_SIDE);
    limits.max_alloc = Some(DECODE_PIXELS * 4 + 16 * 1024 * 1024);
    reader.limits(limits);
    let mut decoder = reader.into_decoder().map_err(failed)?;
    let (width, height) = decoder.dimensions();
    if width == 0 || height == 0 || u64::from(width) * u64::from(height) > DECODE_PIXELS {
        return Err(too_large());
    }
    let orientation = decoder.orientation().map_err(failed)?;
    let mut image = image::DynamicImage::from_decoder(decoder).map_err(failed)?;
    image.apply_orientation(orientation);
    if image.width().max(image.height()) > KEPT_SIDE {
        image = image.resize(KEPT_SIDE, KEPT_SIDE, image::imageops::FilterType::Triangle);
    }
    Ok(image.into_rgba8())
}

fn load(picture: &Picture, storage: &Path) -> Result<image::RgbaImage, String> {
    match picture {
        Picture::None => Err("No picture is chosen.".into()),
        Picture::Own(name) => decode(&read(&folder(storage).join(name))?),
        other => decode(built_in(other).expect("a built-in picture").picture),
    }
}

/// What a texture was prepared for: the palette behind it and how much shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Guard {
    pub base: Base,
    pub visibility: u8,
}

/// Lays `image` on the base background and dims it (dark bases) or lifts it (Light)
/// wherever TEXT would fall below 4.5:1 or MUTED below 3:1 over the see-through panes.
pub fn tone(image: &image::RgbaImage, guard: Guard) -> ColorImage {
    let p = guard.base.palette();
    let light = guard.base.light();
    let v = f32::from(guard.visibility) / 100.0;
    // The pane that lets the most picture through, and its least favourable colour.
    let pane = if light == (luminance(p.bg) < luminance(p.panel)) { p.bg } else { p.panel };
    let bound = if light {
        (4.5 * (luminance(p.text) + 0.05) - 0.05).max(3.0 * (luminance(p.muted) + 0.05) - 0.05)
    } else {
        ((luminance(p.text) + 0.05) / 4.5 - 0.05).min((luminance(p.muted) + 0.05) / 3.0 - 0.05)
    };
    let unit = |c: Color32| c.to_array().map(|x| f32::from(x) / 255.0);
    let (base, pane) = (unit(p.bg), unit(pane));
    // egui blends in gamma space: what shows is pane · (1 − v) + picture · v.
    let behind = |c: [f32; 3]| {
        let blend = |i: usize| pane[i] * (1.0 - v) + c[i] * v;
        0.2126 * lin(blend(0)) + 0.7152 * lin(blend(1)) + 0.0722 * lin(blend(2))
    };
    let pixels = image
        .pixels()
        .map(|px| {
            let a = f32::from(px[3]) / 255.0;
            let c = [0, 1, 2].map(|i| f32::from(px[i]) / 255.0 * a + base[i] * (1.0 - a));
            let adjust = |t: f32| if light { c.map(|x| x + (1.0 - x) * t) } else { c.map(|x| x * (1.0 - t)) };
            let fine = |c| if light { behind(c) >= bound } else { behind(c) <= bound };
            let c = if fine(c) {
                c
            } else {
                // The least change that keeps the text readable.
                let (mut ok, mut bad) = (1.0, 0.0);
                for _ in 0..10 {
                    let mid = (ok + bad) / 2.0;
                    if fine(adjust(mid)) { ok = mid } else { bad = mid }
                }
                adjust(ok)
            };
            let [r, g, b] = c.map(|x| (x * 255.0).round().clamp(0.0, 255.0) as u8);
            Color32::from_rgb(r, g, b)
        })
        .collect();
    ColorImage::new([image.width() as usize, image.height() as usize], pixels)
}

/// sRGB to linear light, from a table.
fn lin(x: f32) -> f32 {
    static TABLE: OnceLock<Vec<f32>> = OnceLock::new();
    let table = TABLE.get_or_init(|| (0..=1024).map(|i| linear(i as f32 / 1024.0)).collect());
    table[(x.clamp(0.0, 1.0) * 1024.0).round() as usize]
}

/// The picture's most common vivid hue as a bright colour, for the From picture
/// accent; None for a picture without vivid colours.
pub fn vivid(image: &image::RgbaImage) -> Option<Color32> {
    // Weight, weighted hue and weighted saturation per 15° of hue.
    let mut bins = [(0.0_f32, 0.0_f32, 0.0_f32); 24];
    let step = (image.width() as usize * image.height() as usize / 40_000).max(1);
    for px in image.pixels().step_by(step) {
        if px[3] < 128 {
            continue;
        }
        let [r, g, b] = [px[0], px[1], px[2]].map(|c| f32::from(c) / 255.0);
        let (max, min) = (r.max(g).max(b), r.min(g).min(b));
        let (s, v) = (if max > 0.0 { (max - min) / max } else { 0.0 }, max);
        if s < 0.3 || v < 0.3 {
            continue;
        }
        let d = max - min;
        let h = 60.0
            * if max == r {
                ((g - b) / d).rem_euclid(6.0)
            } else if max == g {
                (b - r) / d + 2.0
            } else {
                (r - g) / d + 4.0
            };
        let w = s * v;
        let bin = &mut bins[((h / 15.0) as usize).min(23)];
        bin.0 += w;
        bin.1 += h * w;
        bin.2 += s * w;
    }
    let (weight, hue, saturation) = bins.into_iter().max_by(|a, b| a.0.total_cmp(&b.0))?;
    if weight < 40.0 {
        return None;
    }
    let (h, s, v) = (hue / weight, (saturation / weight).clamp(0.5, 0.85), 0.95);
    let c = v * s;
    let x = c * (1.0 - ((h / 60.0).rem_euclid(2.0) - 1.0).abs());
    let (r, g, b) = match (h / 60.0) as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let channel = |c: f32| ((c + v - v * s) * 255.0).round().clamp(0.0, 255.0) as u8;
    Some(Color32::from_rgb(channel(r), channel(g), channel(b)))
}

#[derive(Clone)]
struct Source {
    picture: Picture,
    image: Arc<image::RgbaImage>,
    accent: Option<Color32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Want {
    picture: Picture,
    guard: Guard,
}

impl Want {
    fn of(custom: &Custom) -> Self {
        Self {
            picture: custom.picture.clone(),
            guard: Guard { base: custom.base, visibility: custom.visibility },
        }
    }
}

struct Prepared {
    want: Want,
    /// Set when the picture was decoded for this job.
    source: Option<Source>,
    image: ColorImage,
}

/// The picture layer of the main window.
#[derive(Default)]
pub struct Backdrop {
    /// The decoded picture and its vivid colour, so a Visibility or base change only
    /// prepares it again.
    source: Option<Source>,
    shown: Option<(Want, TextureHandle)>,
    /// At most one preparation runs; a newer choice waits for it and starts next.
    job: Option<(Want, mpsc::Receiver<Result<Prepared, String>>)>,
    /// The picture that could not be shown, and why, until another is chosen.
    failure: Option<(Picture, String)>,
    tiles: HashMap<&'static str, TextureHandle>,
}

impl Backdrop {
    /// Keeps the texture in step with the Custom theme `custom` (None while another
    /// theme shows). Call once a frame; returns the picture's vivid colour once known.
    pub fn update(&mut self, ctx: &Context, custom: Option<&Custom>, storage: &Path) -> Option<Color32> {
        let Some(custom) = custom.filter(|c| c.picture != Picture::None) else {
            self.source = None;
            self.shown = None;
            self.job = None;
            self.failure = None;
            return None;
        };
        if let Some((_, receiver)) = &self.job {
            let finished = match receiver.try_recv() {
                Ok(result) => Some(result),
                Err(mpsc::TryRecvError::Empty) => None,
                Err(mpsc::TryRecvError::Disconnected) => Some(Err("The picture could not be prepared.".into())),
            };
            if let Some(result) = finished {
                let (want, _) = self.job.take().expect("a job");
                match result {
                    Ok(prepared) => {
                        if let Some(source) = prepared.source {
                            self.source = Some(source);
                        }
                        let texture = ctx.load_texture("theme-picture", prepared.image, TextureOptions::LINEAR);
                        self.shown = Some((prepared.want, texture));
                    }
                    Err(message) => self.failure = Some((want.picture, message)),
                }
            }
        }
        if self.failure.as_ref().is_some_and(|(picture, _)| *picture != custom.picture) {
            self.failure = None;
        }
        if !self.shows(custom) && self.failure.is_none() && self.job.is_none() {
            self.start(ctx, Want::of(custom), storage);
        }
        self.source.as_ref().filter(|s| s.picture == custom.picture).and_then(|s| s.accent)
    }

    fn start(&mut self, ctx: &Context, want: Want, storage: &Path) {
        let source = self.source.clone().filter(|s| s.picture == want.picture);
        let (sender, receiver) = mpsc::channel();
        let (job, storage, ctx) = (want.clone(), storage.to_path_buf(), ctx.clone());
        let spawned = std::thread::Builder::new().name("theme-picture".into()).spawn(move || {
            let _ = sender.send(prepare(job, source, &storage));
            ctx.request_repaint();
        });
        match spawned {
            Ok(_) => self.job = Some((want, receiver)),
            Err(e) => self.failure = Some((want.picture, format!("The picture could not be prepared: {e}."))),
        }
    }

    /// Why `picture` is not showing, after it failed to load.
    pub fn problem(&self, picture: &Picture) -> Option<&str> {
        self.failure.as_ref().filter(|(p, _)| p == picture).map(|(_, message)| message.as_str())
    }

    /// Whether a picture is being read or prepared.
    pub fn busy(&self) -> bool {
        self.job.is_some()
    }

    /// Whether the picture on screen is `custom`'s, prepared for its base and Visibility.
    pub fn shows(&self, custom: &Custom) -> bool {
        self.shown.as_ref().is_some_and(|(shown, _)| *shown == Want::of(custom))
    }

    /// Paints the picture over the base background of `rect`, below the panes.
    pub fn paint(&self, painter: &Painter, rect: Rect, custom: &Custom) {
        let Some((shown, texture)) = &self.shown else { return };
        if shown.picture != custom.picture {
            return;
        }
        let size = texture.size_vec2();
        if size.x < 1.0 || size.y < 1.0 {
            return;
        }
        let scale = match custom.size {
            Size::Fill => (rect.width() / size.x).max(rect.height() / size.y),
            Size::Fit => (rect.width() / size.x).min(rect.height() / size.y),
        };
        let target = place(rect, size * scale, custom.position);
        // A fitted picture narrower than the window fades into the theme on its inner sides.
        let narrower = custom.size == Size::Fit && target.width() < rect.width() - 1.0;
        let fade_left = narrower && custom.position != Position::Left;
        let fade_right = narrower && custom.position != Position::Right;
        let mut mesh = Mesh::with_texture(texture.id());
        let stops = [(0.0, fade_left), (0.35, false), (0.65, false), (1.0, fade_right)];
        for (u, faded) in stops {
            let x = target.left() + u * target.width();
            let color = if faded { Color32::TRANSPARENT } else { Color32::WHITE };
            for (y, v) in [(target.top(), 0.0), (target.bottom(), 1.0)] {
                mesh.vertices.push(epaint::Vertex { pos: pos2(x, y), uv: pos2(u, v), color });
            }
        }
        for i in 0..3 {
            let k = i * 2;
            mesh.add_triangle(k, k + 1, k + 2);
            mesh.add_triangle(k + 1, k + 3, k + 2);
        }
        painter.with_clip_rect(rect.intersect(painter.clip_rect())).add(Shape::mesh(mesh));
    }

    /// The Preferences tile picture for `picture` and its size: Rocket's own, or the
    /// user's once shown.
    pub fn tile(&mut self, ctx: &Context, picture: &Picture) -> Option<(TextureId, Vec2)> {
        if let Picture::Own(_) = picture {
            return self
                .shown
                .as_ref()
                .filter(|(shown, _)| shown.picture == *picture)
                .map(|(_, t)| (t.id(), t.size_vec2()));
        }
        let &BuiltIn { key, tile: bytes, .. } = built_in(picture)?;
        if !self.tiles.contains_key(key) {
            let image = image::load_from_memory_with_format(bytes, image::ImageFormat::Jpeg).ok()?.into_rgba8();
            let size = [image.width() as usize, image.height() as usize];
            let texture = ctx.load_texture(format!("theme-tile-{key}"), ColorImage::from_rgba_unmultiplied(size, image.as_raw()), TextureOptions::LINEAR);
            self.tiles.insert(key, texture);
        }
        self.tiles.get(key).map(|t| (t.id(), t.size_vec2()))
    }

    /// The vivid colour of `picture`, once it has been read.
    pub fn accent(&self, picture: &Picture) -> Option<Color32> {
        self.source.as_ref().filter(|s| s.picture == *picture).and_then(|s| s.accent)
    }
}

/// `size` placed in `rect`: vertically centred, horizontally by `position`.
fn place(rect: Rect, size: Vec2, position: Position) -> Rect {
    let left = match position {
        Position::Left => rect.left(),
        Position::Centre => rect.center().x - size.x / 2.0,
        Position::Right => rect.right() - size.x,
    };
    Rect::from_min_size(pos2(left, rect.center().y - size.y / 2.0), size)
}

fn prepare(want: Want, source: Option<Source>, storage: &Path) -> Result<Prepared, String> {
    let (source, fresh) = match source {
        Some(source) => (source, false),
        None => {
            let image = load(&want.picture, storage)?;
            let accent = match built_in(&want.picture) {
                Some(&BuiltIn { accent: [r, g, b], .. }) => Some(Color32::from_rgb(r, g, b)),
                None => vivid(&image),
            };
            (Source { picture: want.picture.clone(), image: Arc::new(image), accent }, true)
        }
    };
    let image = tone(&source.image, want.guard);
    Ok(Prepared { want, source: fresh.then_some(source), image })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::style::Palette;
    use crate::theme::contrast;

    #[test]
    fn toned_pictures_keep_text_readable_over_the_panes() {
        // White, black and saturated pixels, plus a half-transparent one.
        let mut image = image::RgbaImage::new(6, 1);
        for (x, px) in [[255, 255, 255, 255], [0, 0, 0, 255], [255, 0, 180, 255], [0, 255, 120, 255], [40, 120, 255, 255], [255, 255, 255, 128]].into_iter().enumerate() {
            image.put_pixel(x as u32, 0, image::Rgba(px));
        }
        for base in Base::ALL {
            let p = base.palette();
            for visibility in [*super::super::VISIBILITY.start(), 30, *super::super::VISIBILITY.end()] {
                let toned = tone(&image, Guard { base, visibility });
                let look = super::super::resolve(
                    &super::super::Settings {
                        choice: super::super::Choice::Custom,
                        custom: Custom { base, visibility, accent: super::super::Accent::Colour([34, 195, 238]), ..Default::default() },
                    },
                    None,
                    None,
                );
                for px in &toned.pixels {
                    // Every pane over this pixel, as egui blends it.
                    for pane in [look.palette.pane_bg, look.palette.pane_panel, look.palette.pane_deep] {
                        let shown = px.blend(pane);
                        let (text, muted) = (contrast(p.text, shown), contrast(p.muted, shown));
                        assert!(text >= 4.4 && muted >= 2.9, "{base:?} at {visibility}%: {px:?} under {pane:?} gives text {text:.2}, muted {muted:.2}");
                    }
                }
            }
        }
        assert_eq!(Palette::NAVY, Base::Default.palette());
    }

    #[test]
    fn vivid_finds_the_pictures_hue_and_ignores_grey() {
        let pink = image::RgbaImage::from_fn(300, 200, |x, _| if x < 200 { image::Rgba([200, 60, 150, 255]) } else { image::Rgba([30, 30, 40, 255]) });
        let found = vivid(&pink).expect("a vivid colour");
        let [r, g, b, _] = found.to_array();
        assert!(r > b && b > g, "pink stays pink: {found:?}");
        let grey = image::RgbaImage::from_pixel(300, 200, image::Rgba([128, 128, 128, 255]));
        assert_eq!(vivid(&grey), None);
    }

    #[test]
    fn built_in_pictures_decode_at_their_size() {
        for picture in Picture::BUILT_IN {
            let image = load(&picture, Path::new("unused")).unwrap();
            assert_eq!((image.width(), image.height()), (1920, 1200), "{picture:?}");
        }
    }

    #[test]
    fn pictures_fill_or_fit_from_the_chosen_side() {
        let window = Rect::from_min_size(pos2(0.0, 0.0), vec2(1000.0, 500.0));
        let right = place(window, vec2(300.0, 500.0), Position::Right);
        assert_eq!((right.left(), right.right()), (700.0, 1000.0));
        let left = place(window, vec2(300.0, 500.0), Position::Left);
        assert_eq!(left.left(), 0.0);
        let centre = place(window, vec2(1200.0, 600.0), Position::Centre);
        assert_eq!((centre.left(), centre.top()), (-100.0, -50.0));
    }

    #[test]
    fn chosen_pictures_are_checked_kept_and_replaced() {
        let dir = std::env::temp_dir().join(format!("rocket-theme-{}-{:?}", std::process::id(), std::thread::current().id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let png = |name: &str, w: u32, h: u32| {
            let path = dir.join(name);
            image::RgbaImage::from_pixel(w, h, image::Rgba([10, 200, 90, 255])).save(&path).unwrap();
            path
        };
        let first = import(&png("a.png", 3000, 1500), &dir).unwrap();
        assert!(own_name(&first));
        let kept = image::open(folder(&dir).join(&first)).unwrap();
        assert_eq!((kept.width(), kept.height()), (KEPT_SIDE, KEPT_SIDE / 2), "large pictures are scaled down");
        let leftover = folder(&dir).join("picture-0000000000000001.png.999.tmp");
        std::fs::write(&leftover, b"half written").unwrap();
        let second = import(&png("b.png", 40, 20), &dir).unwrap();
        assert_ne!(first, second);
        assert!(!folder(&dir).join(&first).exists(), "the picture chosen before is removed");
        assert!(!leftover.exists(), "a half-written copy is removed");
        std::fs::write(dir.join("c.png"), b"not a picture").unwrap();
        assert_eq!(import(&dir.join("c.png"), &dir), Err("Choose a PNG, JPG or WebP picture.".into()));
        let gif = dir.join("d.gif");
        std::fs::write(&gif, b"GIF89a\x01\x00\x01\x00\x00\x00\x00;").unwrap();
        assert!(import(&gif, &dir).is_err());
        assert!(import(&dir.join("missing.png"), &dir).unwrap_err().contains("missing"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
