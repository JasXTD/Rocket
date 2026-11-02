//! Local Windows fonts supplement egui's embedded fonts. Nothing is downloaded.
use eframe::egui::{FontData, FontDefinitions, FontFamily};
use std::{path::Path, sync::OnceLock};

pub(crate) fn definitions() -> FontDefinitions {
    // Reopening an MCP window reuses the same font bytes. FontDefinitions clones
    // Arc<FontData>, not the multi-megabyte CJK files; no font I/O runs per frame.
    static FONTS: OnceLock<FontDefinitions> = OnceLock::new();
    FONTS
        .get_or_init(|| {
            let windows = std::env::var_os("WINDIR")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| "C:/Windows".into());
            from_directory(&windows.join("Fonts"))
        })
        .clone()
}

fn from_directory(directory: &Path) -> FontDefinitions {
    let mut fonts = FontDefinitions::default();
    if let Some(name) = load_first(&mut fonts, directory, &["segoeui.ttf"]) {
        fonts
            .families
            .get_mut(&FontFamily::Proportional)
            .unwrap()
            .insert(0, name.clone());
        fonts
            .families
            .get_mut(&FontFamily::Monospace)
            .unwrap()
            .push(name);
    }
    // One installed family per writing system, rather than every Windows font.
    // TTC collections use their first face (FontData's default index, 0).
    for candidates in [
        &["msyh.ttc", "msjh.ttc", "simsun.ttc"][..],
        &["YuGothM.ttc", "meiryo.ttc", "msgothic.ttc"][..],
        &["malgun.ttf"][..],
        &["seguiemj.ttf"][..],
        &["seguisym.ttf"][..],
    ] {
        if let Some(name) = load_first(&mut fonts, directory, candidates) {
            for family in fonts.families.values_mut() {
                family.push(name.clone());
            }
        }
    }
    // The embedded emoji fallbacks predate many ZWJ/skin-tone/keycap sequences.
    // Terminal wide clusters need one face that can shape the whole sequence,
    // including its ASCII digit or variation selector, before those fallbacks.
    let mut terminal_wide = fonts.families[&FontFamily::Monospace].clone();
    if fonts.font_data.contains_key("seguiemj.ttf") {
        terminal_wide.retain(|name| name != "seguiemj.ttf");
        terminal_wide.insert(0, "seguiemj.ttf".into());
    }
    fonts
        .families
        .insert(FontFamily::Name("terminal-wide".into()), terminal_wide);
    let mut bold = fonts.families[&FontFamily::Proportional].clone();
    if let Some(name) = load_first(&mut fonts, directory, &["seguisb.ttf"]) {
        bold.insert(0, name);
    }
    fonts
        .families
        .insert(FontFamily::Name("semibold".into()), bold);
    fonts
}

fn load_first(
    fonts: &mut FontDefinitions,
    directory: &Path,
    candidates: &[&str],
) -> Option<String> {
    for candidate in candidates {
        if let Ok(bytes) = std::fs::read(directory.join(candidate)) {
            fonts
                .font_data
                .insert((*candidate).into(), FontData::from_owned(bytes).into());
            return Some((*candidate).into());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_system_fonts_keep_embedded_fonts_and_heading_family() {
        // A file cannot contain a Fonts directory, regardless of test ordering.
        let definitions = from_directory(&std::env::current_exe().unwrap().join("Fonts"));
        let ctx = eframe::egui::Context::default();
        ctx.set_fonts(definitions);
        let mut output = ctx.run_ui(Default::default(), |ui| {
            ui.ctx().fonts_mut(|fonts| {
                for family in [
                    FontFamily::Proportional,
                    FontFamily::Monospace,
                    FontFamily::Name("semibold".into()),
                ] {
                    let mut font = fonts.fonts.font(&family);
                    assert!(
                        "Rocket 0123"
                            .chars()
                            .all(|c| font.characters().contains_key(&c))
                    );
                }
            });
        });
        output.textures_delta.clear();
    }
}
