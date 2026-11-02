use eframe::egui::ColorImage;
use std::{io::Write, path::Path};

pub fn save_bmp(path: &Path, image: &ColorImage) -> std::io::Result<()> {
    let [width, height] = image.size;
    let size = 54 + width * height * 4;
    let mut header = [0_u8; 54];
    header[0..2].copy_from_slice(b"BM");
    header[2..6].copy_from_slice(&(size as u32).to_le_bytes());
    header[10..14].copy_from_slice(&54_u32.to_le_bytes());
    header[14..18].copy_from_slice(&40_u32.to_le_bytes());
    header[18..22].copy_from_slice(&(width as i32).to_le_bytes());
    header[22..26].copy_from_slice(&(-(height as i32)).to_le_bytes());
    header[26..28].copy_from_slice(&1_u16.to_le_bytes());
    header[28..30].copy_from_slice(&32_u16.to_le_bytes());
    let mut file = std::io::BufWriter::new(std::fs::File::create(path)?);
    file.write_all(&header)?;
    for pixel in &image.pixels {
        let [r, g, b, _] = pixel.to_array();
        file.write_all(&[b, g, r, 255])?;
    }
    file.flush()
}
