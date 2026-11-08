//! Generate Apple's iconset from the same artwork as the Windows/window icon.
//! Usage: cargo run -p rocket-ui --example macos_icon -- NEW_DIRECTORY.iconset
use image::{ExtendedColorType, ImageEncoder, codecs::png::PngEncoder};
use rocket_ui::app_icon::render;
use std::{collections::BTreeMap, fs, io, path::PathBuf};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let output = args
        .next()
        .map(PathBuf::from)
        .filter(|_| args.next().is_none())
        .ok_or("usage: macos_icon NEW_DIRECTORY.iconset")?;
    if output.extension().is_none_or(|s| s != "iconset") {
        return Err("output must be a new .iconset directory".into());
    }
    fs::create_dir(&output)?; // Never replace existing artwork or a link.
    let mut images = BTreeMap::new();
    for points in [16, 32, 128, 256, 512] {
        for scale in [1, 2] {
            let side = points * scale;
            let png = images.entry(side).or_insert_with(|| {
                let mut png = Vec::new();
                PngEncoder::new(&mut png)
                    .write_image(&render(side).rgba, side, side, ExtendedColorType::Rgba8)
                    .expect("encode first-party icon");
                png
            });
            let suffix = if scale == 1 { "" } else { "@2x" };
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(output.join(format!("icon_{points}x{points}{suffix}.png")))?;
            io::Write::write_all(&mut file, png)?;
        }
    }
    println!(
        "Created ten icon images, 16–1024 pixels, in {}",
        output.display()
    );
    Ok(())
}
