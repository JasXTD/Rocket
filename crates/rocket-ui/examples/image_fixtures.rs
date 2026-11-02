//! Synthetic images used only by the isolated localhost-MCP acceptance runner.
use image::{ImageEncoder, ImageFormat, Rgba, RgbaImage};
use std::{fs, io::Cursor, path::PathBuf};
fn main() {
    let target = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .expect("new fixture image folder"),
    );
    let parent = target.parent().unwrap().canonicalize().unwrap();
    let boundary = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../work/mcp-fixtures")
        .canonicalize()
        .unwrap();
    assert!(parent.starts_with(boundary));
    fs::create_dir(&target).expect("fixture image folder must not exist");
    for (name, w, h, updated, format) in [
        ("before.png", 320, 200, false, ImageFormat::Png),
        ("after.png", 400, 260, true, ImageFormat::Png),
        ("external.png", 280, 180, true, ImageFormat::Png),
        ("sample.bmp", 120, 80, true, ImageFormat::Bmp),
        ("sample.jpg", 150, 100, true, ImageFormat::Jpeg),
        ("wide.png", 4097, 1, true, ImageFormat::Png),
        ("pixels.png", 3000, 3000, true, ImageFormat::Png),
    ] {
        let pixels = RgbaImage::from_fn(w, h, |x, y| {
            let dx = x as f32 / w as f32 - 0.5;
            let dy = y as f32 / h as f32 - 0.5;
            if dx * dx + dy * dy < if updated { 0.15 } else { 0.19 } {
                Rgba(if updated {
                    [34, 155, 218, 210]
                } else {
                    [62, 111, 160, 230]
                })
            } else {
                Rgba([0, 0, 0, 0])
            }
        });
        let image = image::DynamicImage::ImageRgba8(pixels);
        let mut out = Cursor::new(Vec::new());
        if format == ImageFormat::Jpeg {
            image.to_rgb8().write_to(&mut out, format).unwrap()
        } else {
            image.write_to(&mut out, format).unwrap()
        };
        fs::write(target.join(name), out.into_inner()).unwrap();
    }
    let pixels = image::RgbImage::from_pixel(150, 100, image::Rgb([20, 155, 220]));
    let mut bytes = Vec::new();
    let mut encoder = image::codecs::jpeg::JpegEncoder::new(&mut bytes);
    encoder
        .set_exif_metadata(vec![
            b'I', b'I', 42, 0, 8, 0, 0, 0, 1, 0, 0x12, 1, 3, 0, 1, 0, 0, 0, 6, 0, 0, 0, 0, 0, 0, 0,
        ])
        .unwrap();
    encoder.encode_image(&pixels).unwrap();
    fs::write(target.join("rotated.jpg"), bytes).unwrap();
}
