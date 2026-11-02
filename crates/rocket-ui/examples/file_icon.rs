//! Writes `src/rocket.ico` and the README's PNG logo from the drawing in
//! `app_icon.rs`: `cargo run -p rocket-ui --example file_icon`. Every image is stored as
//! a PNG, which Windows reads in icons of any size.
use image::{
    ExtendedColorType, ImageEncoder,
    codecs::png::{CompressionType, FilterType, PngEncoder},
};
use rocket_ui::app_icon::{FILE_SIZES, render};
use std::{fs, path::PathBuf};

fn main() {
    let images: Vec<Vec<u8>> = FILE_SIZES
        .iter()
        .map(|&size| {
            assert!(size <= 256, "an icon image is at most 256 pixels");
            let mut png = Vec::new();
            PngEncoder::new_with_quality(&mut png, CompressionType::Best, FilterType::Adaptive)
                .write_image(&render(size).rgba, size, size, ExtendedColorType::Rgba8)
                .expect("encode the icon image");
            png
        })
        .collect();
    // ICONDIR, one 16-byte ICONDIRENTRY per image, then the images.
    let mut ico = Vec::new();
    for word in [0, 1, images.len() as u16] {
        ico.extend(word.to_le_bytes());
    }
    let mut offset = 6 + 16 * images.len();
    for (&size, png) in FILE_SIZES.iter().zip(&images) {
        // Width and height in one byte each, 256 as 0; no palette; 1 plane, 32 bits.
        let side = (size % 256) as u8;
        ico.extend([side, side, 0, 0, 1, 0, 32, 0]);
        ico.extend((png.len() as u32).to_le_bytes());
        ico.extend((offset as u32).to_le_bytes());
        offset += png.len();
    }
    for png in &images {
        ico.extend(png);
    }
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/rocket.ico");
    fs::write(&path, &ico).expect("write rocket.ico");
    println!("Wrote {} ({} bytes)", path.display(), ico.len());
    let logo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/images/rocket-logo.png");
    let png = &images[FILE_SIZES
        .iter()
        .position(|&size| size == 256)
        .expect("256 px logo")];
    fs::write(&logo, png).expect("write the README logo");
    println!("Wrote {} ({} bytes)", logo.display(), png.len());
}
