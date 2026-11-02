//! The window and taskbar icon: an upright, faceted rocket on a navy rounded tile.
//! The brand colors stay fixed across themes; monochrome UI marks use the same
//! geometry with a tint. eframe scales the window icon to the sizes Windows asks for.
//!
//! The executables' file icon, `rocket.ico`, holds the same drawing at [`FILE_SIZES`].
//! The build scripts embed it; after changing the drawing or the palette, regenerate it
//! with `cargo run -p rocket-ui --example file_icon` (a test fails until then). That
//! command also refreshes `docs/images/rocket-logo.png` for the README.
use crate::style::Palette;
use eframe::egui::{self, Color32, IconData};

/// Side of the window icon in pixels.
const SIZE: u32 = 128;
/// Sides of the images in `rocket.ico`, smallest first.
pub const FILE_SIZES: [u32; 6] = [16, 24, 32, 48, 64, 256];
/// Samples per pixel along each axis where a pixel straddles an edge.
const SAMPLES: u32 = 4;
/// Tile inset and corner radius, as fractions of the image side.
const TILE_INSET: f32 = 0.03;
const TILE_RADIUS: f32 = 0.22;
const BRAND: Palette = Palette::NAVY;

struct Polygon {
    points: Vec<[f32; 2]>,
    /// Min x, min y, max x, max y.
    bounds: [f32; 4],
}

impl Polygon {
    fn new(points: Vec<[f32; 2]>) -> Self {
        let mut bounds = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
        for [x, y] in &points {
            bounds = [
                bounds[0].min(*x),
                bounds[1].min(*y),
                bounds[2].max(*x),
                bounds[3].max(*y),
            ];
        }
        Self { points, bounds }
    }

    fn contains(&self, p: [f32; 2]) -> bool {
        let [x0, y0, x1, y1] = self.bounds;
        if p[0] < x0 || p[0] > x1 || p[1] < y0 || p[1] > y1 {
            return false;
        }
        // Even-odd ray casting.
        let mut inside = false;
        let mut j = self.points.len() - 1;
        for (i, a) in self.points.iter().enumerate() {
            let b = self.points[j];
            if (a[1] > p[1]) != (b[1] > p[1])
                && p[0] < (b[0] - a[0]) * (p[1] - a[1]) / (b[1] - a[1]) + a[0]
            {
                inside = !inside;
            }
            j = i;
        }
        inside
    }
}

/// The rocket's parts, front to back, in image coordinates (0..1, y down).
fn parts() -> Vec<(Polygon, Color32)> {
    // Broad facets, swept fins and a detached exhaust read clearly at 16 px.
    // Every part is convex so the raster and egui painters share these points.
    vec![
        (
            Polygon::new(vec![[0.50, 0.14], [0.50, 0.58], [0.40, 0.66], [0.36, 0.40]]),
            BRAND.text,
        ),
        (
            Polygon::new(vec![[0.50, 0.14], [0.64, 0.40], [0.60, 0.66], [0.50, 0.58]]),
            BRAND.accent,
        ),
        (
            Polygon::new(vec![
                [0.345, 0.46],
                [0.39, 0.67],
                [0.225, 0.755],
                [0.255, 0.565],
            ]),
            BRAND.accent,
        ),
        (
            Polygon::new(vec![
                [0.655, 0.46],
                [0.745, 0.565],
                [0.775, 0.755],
                [0.61, 0.67],
            ]),
            BRAND.text,
        ),
        (
            Polygon::new(vec![
                [0.50, 0.67],
                [0.56, 0.72],
                [0.50, 0.875],
                [0.44, 0.72],
            ]),
            BRAND.amber,
        ),
    ]
}

/// Paint the shared mark without a tile. Menu glyphs supply a theme-aware tint;
/// About keeps the brand colors on a navy tile.
pub fn paint_mark(painter: &egui::Painter, rect: egui::Rect, tint: Option<Color32>) {
    for (polygon, color) in parts().into_iter().rev() {
        let points = polygon
            .points
            .into_iter()
            .map(|[x, y]| {
                egui::pos2(
                    rect.left() + x * rect.width(),
                    rect.top() + y * rect.height(),
                )
            })
            .collect();
        painter.add(egui::Shape::convex_polygon(
            points,
            tint.unwrap_or(color),
            egui::Stroke::NONE,
        ));
    }
}

/// Inside the rounded tile, by the distance to a rounded square.
fn in_tile([x, y]: [f32; 2]) -> bool {
    let half = 0.5 - TILE_INSET - TILE_RADIUS;
    let dx = ((x - 0.5).abs() - half).max(0.0);
    let dy = ((y - 0.5).abs() - half).max(0.0);
    dx * dx + dy * dy <= TILE_RADIUS * TILE_RADIUS
}

/// The tile shades diagonally from the selection blue to the deepest surface.
fn tile_color([x, y]: [f32; 2]) -> [f32; 3] {
    let t = ((x + y) / 2.0).clamp(0.0, 1.0);
    let [r0, g0, b0, _] = BRAND.selected.to_array();
    let [r1, g1, b1, _] = BRAND.deep.to_array();
    let mix = |a: u8, b: u8| a as f32 + (b as f32 - a as f32) * t;
    [mix(r0, r1), mix(g0, g1), mix(b0, b1)]
}

/// Rocket's window icon as straight (not premultiplied) RGBA.
pub fn icon() -> IconData {
    render(SIZE)
}

/// The icon drawn `size` pixels square, as straight (not premultiplied) RGBA.
pub fn render(size: u32) -> IconData {
    let parts = parts();
    // What covers a point: a part index, `parts.len()` for the bare tile, or nothing.
    let hit = |p: [f32; 2]| {
        in_tile(p).then(|| {
            parts
                .iter()
                .position(|(shape, _)| shape.contains(p))
                .unwrap_or(parts.len())
        })
    };
    let color = |p: [f32; 2], part: usize| {
        parts.get(part).map_or_else(
            || tile_color(p),
            |(_, c)| [c.r() as f32, c.g() as f32, c.b() as f32],
        )
    };
    let side = size as f32;
    // A pixel whose four corners agree is drawn from its center; only pixels that
    // straddle an edge are supersampled.
    let n = size as usize + 1;
    let corners: Vec<_> = (0..n * n)
        .map(|i| hit([(i % n) as f32 / side, (i / n) as f32 / side]))
        .collect();
    let mut rgba = Vec::with_capacity((size * size * 4) as usize);
    for py in 0..size as usize {
        for px in 0..size as usize {
            let corner = corners[py * n + px];
            let center = [(px as f32 + 0.5) / side, (py as f32 + 0.5) / side];
            if [
                corners[py * n + px + 1],
                corners[(py + 1) * n + px],
                corners[(py + 1) * n + px + 1],
            ]
            .iter()
            .all(|c| *c == corner)
            {
                match corner {
                    None => rgba.extend([0, 0, 0, 0]),
                    Some(part) => {
                        let [r, g, b] = color(center, part);
                        rgba.extend([r.round() as u8, g.round() as u8, b.round() as u8, 255]);
                    }
                }
                continue;
            }
            let (mut sum, mut covered) = ([0.0_f32; 3], 0_u32);
            for sy in 0..SAMPLES {
                for sx in 0..SAMPLES {
                    let p = [
                        (px as f32 + (sx as f32 + 0.5) / SAMPLES as f32) / side,
                        (py as f32 + (sy as f32 + 0.5) / SAMPLES as f32) / side,
                    ];
                    if let Some(part) = hit(p) {
                        for (total, channel) in sum.iter_mut().zip(color(p, part)) {
                            *total += channel;
                        }
                        covered += 1;
                    }
                }
            }
            // Average only the covered samples: the edge keeps its color and the
            // coverage becomes the alpha.
            let average = |total: f32| (total / covered.max(1) as f32).round() as u8;
            rgba.extend([
                average(sum[0]),
                average(sum[1]),
                average(sum[2]),
                (covered * 255 / (SAMPLES * SAMPLES)) as u8,
            ]);
        }
    }
    IconData {
        rgba,
        width: size,
        height: size,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icon_is_a_faceted_rocket_on_a_rounded_tile() {
        let icon = icon();
        assert_eq!((icon.width, icon.height), (SIZE, SIZE));
        assert_eq!(icon.rgba.len(), (SIZE * SIZE * 4) as usize);
        let pixel = |x: u32, y: u32| {
            let i = ((y * SIZE + x) * 4) as usize;
            [
                icon.rgba[i],
                icon.rgba[i + 1],
                icon.rgba[i + 2],
                icon.rgba[i + 3],
            ]
        };
        for (x, y) in [(0, 0), (SIZE - 1, 0), (0, SIZE - 1), (SIZE - 1, SIZE - 1)] {
            assert_eq!(
                pixel(x, y)[3],
                0,
                "corner {x},{y} is outside the rounded tile"
            );
        }
        assert_eq!(
            pixel(SIZE / 2, 8)[3],
            255,
            "the tile is opaque inside its edge"
        );
        // Both facets and the exhaust cover a visible area.
        let count = |color: Color32| {
            icon.rgba
                .chunks(4)
                .filter(|p| p[3] == 255 && p[..3] == color.to_array()[..3])
                .count()
        };
        let area = (SIZE * SIZE) as usize;
        assert!(
            count(BRAND.text) > area / 25,
            "light facets {}",
            count(BRAND.text)
        );
        assert!(
            count(BRAND.accent) > area / 25,
            "cyan facets {}",
            count(BRAND.accent)
        );
        assert!(
            count(BRAND.amber) > area / 200,
            "exhaust {}",
            count(BRAND.amber)
        );
    }

    #[test]
    fn file_icon_holds_the_drawing_at_each_size() {
        let file = include_bytes!("rocket.ico");
        let word = |at: usize| u16::from_le_bytes([file[at], file[at + 1]]);
        let dword = |at: usize| u32::from_le_bytes(file[at..at + 4].try_into().unwrap()) as usize;
        assert_eq!([word(0), word(2), word(4)], [0, 1, FILE_SIZES.len() as u16]);
        for (i, size) in FILE_SIZES.into_iter().enumerate() {
            let entry = 6 + 16 * i;
            // Width and height (256 as 0), no palette, 1 plane, 32 bits per pixel.
            let side = (size % 256) as u8;
            assert_eq!(file[entry..entry + 8], [side, side, 0, 0, 1, 0, 32, 0]);
            let (len, offset) = (dword(entry + 8), dword(entry + 12));
            let image = image::load_from_memory_with_format(
                &file[offset..offset + len],
                image::ImageFormat::Png,
            )
            .unwrap()
            .into_rgba8();
            assert!(
                image.dimensions() == (size, size) && image.into_raw() == render(size).rgba,
                "rocket.ico's {size} px image is not the drawing; \
                 run `cargo run -p rocket-ui --example file_icon`"
            );
            if size == 256 {
                assert_eq!(
                    &file[offset..offset + len],
                    include_bytes!("../../../docs/images/rocket-logo.png"),
                    "the README logo must match the 256 px file icon"
                );
            }
        }
    }
}
