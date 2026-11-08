//! Installed fonts supplement egui; Linux also embeds a pinned CJK fallback.
//! No font is downloaded at runtime.
use crate::i18n::Han;
use eframe::egui::{FontData, FontDefinitions, FontFamily, FontTweak, epaint::text::VariationCoords};
use std::{path::PathBuf, sync::OnceLock};

/// The fonts in the order the interface language reads Han characters: the same
/// character takes different forms in Chinese and Japanese fonts (直, 骨, 角).
pub(crate) fn definitions() -> FontDefinitions {
    let mut fonts = loaded();
    let han = crate::i18n::han();
    han_first(&mut fonts, &PLAN, han);
    #[cfg(target_os = "linux")]
    linux_cjk::select(&mut fonts, han);
    fonts
}

fn loaded() -> FontDefinitions {
    // Reopening an MCP window reuses the same font bytes. FontDefinitions clones
    // Arc<FontData>, not the multi-megabyte CJK files; no font I/O runs per frame.
    static FONTS: OnceLock<FontDefinitions> = OnceLock::new();
    FONTS
        .get_or_init(|| {
            #[cfg(windows)]
            {
                let windows = std::env::var_os("WINDIR")
                    .map(std::path::PathBuf::from)
                    .unwrap_or_else(|| "C:/Windows".into());
                let directory = windows.join("Fonts");
                build(&PLAN, |file| Some(directory.join(file)))
            }
            #[cfg(not(windows))]
            {
                let installed = system::installed();
                build(&PLAN, |file| installed.get(file).cloned())
            }
        })
        .clone()
}

/// A font file, the face within it for a collection, and, for a variable font, the
/// weight to draw it at.
#[derive(Clone, Copy)]
struct Face {
    file: &'static str,
    weight: Option<f32>,
    /// The face in a TTC collection; 0, the first, for every other file.
    index: u32,
}
const fn face(file: &'static str) -> Face {
    Face { file, weight: None, index: 0 }
}
/// One face of a collection, such as Yu Gothic UI, the second face of YuGothM.ttc.
#[cfg_attr(not(windows), allow(dead_code))]
const fn face_in(file: &'static str, index: u32) -> Face {
    Face { file, weight: None, index }
}
impl Face {
    /// The name the face is loaded under: its file, then its face and weight if any.
    fn name(self) -> String {
        let mut name = self.file.to_owned();
        if self.index > 0 {
            name.push_str(&format!("#{}", self.index));
        }
        if let Some(weight) = self.weight {
            name.push_str(&format!(" {weight}"));
        }
        name
    }
}

/// One platform's font files for each role, each list in order of preference.
struct Plan {
    /// Interface faces, each with the semibold of its own family.
    interface: &'static [(Face, Face)],
    /// One installed family per writing system or symbol set, rather than every font.
    fallbacks: &'static [&'static [Face]],
    /// The fallback that shapes whole emoji sequences in the terminal.
    emoji: usize,
    /// The Chinese, Japanese and Korean fallbacks, which [`han_first`] orders; None when
    /// one collection covers all three.
    han: Option<[usize; 3]>,
}

#[cfg(windows)]
const PLAN: Plan = Plan {
    interface: &[(face("segoeui.ttf"), face("seguisb.ttf"))],
    fallbacks: &[
        &[face("msyh.ttc"), face("msjh.ttc"), face("simsun.ttc")],
        // Yu Gothic UI, the face Windows' Japanese interface pairs with Segoe UI: its kana
        // are proportional and its line metrics match. The collection's first face, Yu
        // Gothic, spaced kana apart and set them above the surrounding Latin text.
        &[face_in("YuGothM.ttc", 1), face("meiryo.ttc"), face("msgothic.ttc")],
        &[face("malgun.ttf")],
        &[face("seguiemj.ttf")],
        &[face("seguisym.ttf")],
    ],
    emoji: 3,
    han: Some([0, 1, 2]),
};

/// Ubuntu Sans ships as one variable file; its weight axis gives both faces.
#[cfg(not(windows))]
const UBUNTU_SANS: &str = "UbuntuSans[wdth,wght].ttf";
#[cfg(not(windows))]
const PLAN: Plan = Plan {
    interface: &[
        (
            Face { file: UBUNTU_SANS, weight: Some(400.0), index: 0 },
            Face { file: UBUNTU_SANS, weight: Some(600.0), index: 0 },
        ),
        (face("NotoSans-Regular.ttf"), face("NotoSans-SemiBold.ttf")),
        (face("Ubuntu-R.ttf"), face("Ubuntu-M.ttf")),
        (face("DejaVuSans.ttf"), face("DejaVuSans-Bold.ttf")),
        (face("LiberationSans-Regular.ttf"), face("LiberationSans-Bold.ttf")),
    ],
    fallbacks: &[
        // One collection covers Chinese, Japanese and Korean; its first face is Japanese.
        &[face("NotoSansCJK-Regular.ttc"), face("DroidSansFallbackFull.ttf"), face("wqy-microhei.ttc"), face("wqy-zenhei.ttc")],
        // Outline emoji only: colour-bitmap fonts such as Noto Color Emoji would draw blanks.
        &[face("NotoEmoji-Regular.ttf"), face("Symbola.ttf"), face("Symbola_hint.ttf")],
        &[face("NotoSansSymbols2-Regular.ttf"), face("DejaVuSans.ttf")],
    ],
    emoji: 1,
    han: None,
};

/// Moves the face of the wanted Han forms just before the first of the other Han faces
/// in every family, so shared characters take those forms. All stay loaded; only the
/// order changes. Chinese is the plan's own order.
fn han_first(fonts: &mut FontDefinitions, plan: &Plan, han: Han) {
    let Some(roles) = plan.han else { return };
    let loaded = |role: usize| plan.fallbacks[role].iter().map(|f| f.name()).find(|name| fonts.font_data.contains_key(name));
    let faces = roles.map(loaded);
    let Some(wanted) = faces[han as usize].clone() else { return };
    for family in fonts.families.values_mut() {
        let first = family.iter().position(|name| faces.iter().flatten().any(|face| face == name));
        let at = family.iter().position(|name| *name == wanted);
        if let (Some(first), Some(at)) = (first, at)
            && at > first
        {
            let name = family.remove(at);
            family.insert(first, name);
        }
    }
}

fn build(plan: &Plan, find: impl Fn(&str) -> Option<PathBuf>) -> FontDefinitions {
    let mut fonts = FontDefinitions::default();
    let interface = plan
        .interface
        .iter()
        .find_map(|(regular, semibold)| Some((load(&mut fonts, &find, *regular)?, *semibold)));
    if let Some((name, _)) = &interface {
        fonts
            .families
            .get_mut(&FontFamily::Proportional)
            .unwrap()
            .insert(0, name.clone());
        fonts
            .families
            .get_mut(&FontFamily::Monospace)
            .unwrap()
            .push(name.clone());
    }
    let mut emoji = None;
    for (role, candidates) in plan.fallbacks.iter().enumerate() {
        let Some(name) = candidates
            .iter()
            .find_map(|candidate| load(&mut fonts, &find, *candidate))
        else {
            continue;
        };
        // A face another role already added keeps its earlier place.
        for family in fonts.families.values_mut() {
            if !family.contains(&name) {
                family.push(name.clone());
            }
        }
        if role == plan.emoji {
            emoji = Some(name);
        }
    }
    // The embedded emoji fallbacks predate many ZWJ/skin-tone/keycap sequences.
    // Terminal wide clusters need one face that can shape the whole sequence,
    // including its ASCII digit or variation selector, before those fallbacks.
    let mut terminal_wide = fonts.families[&FontFamily::Monospace].clone();
    if let Some(name) = emoji {
        terminal_wide.retain(|other| *other != name);
        terminal_wide.insert(0, name);
    }
    fonts
        .families
        .insert(FontFamily::Name("terminal-wide".into()), terminal_wide);
    // Headings use the interface family's semibold; without an interface font, the
    // first family that has one.
    let mut bold = fonts.families[&FontFamily::Proportional].clone();
    let semibold = match interface {
        Some((_, semibold)) => load(&mut fonts, &find, semibold),
        None => plan
            .interface
            .iter()
            .find_map(|(_, semibold)| load(&mut fonts, &find, *semibold)),
    };
    if let Some(name) = semibold {
        bold.insert(0, name);
    }
    fonts
        .families
        .insert(FontFamily::Name("semibold".into()), bold);
    #[cfg(target_os = "linux")]
    linux_cjk::append(&mut fonts);
    fonts
}

#[cfg(target_os = "linux")]
mod linux_cjk {
    use super::*;

    pub(super) const PROPORTIONAL: &str = "rocket-noto-cjk-2.004";
    pub(super) const MONOSPACE: &str = "rocket-noto-mono-cjk-2.004";
    // One immutable collection shared by both faces, with no file I/O or font
    // installation. Include the same bytes in cargo-run and portable builds.
    const BYTES: &[u8] = include_bytes!("../../../runtime/fonts/noto-sans-cjk/2.004/NotoSansCJK-Regular.ttc");

    pub(super) fn append(fonts: &mut FontDefinitions) {
        for name in [PROPORTIONAL, MONOSPACE] {
            fonts.font_data.insert(name.into(), FontData::from_static(BYTES).into());
        }
        for (family, names) in &mut fonts.families {
            let mono = *family == FontFamily::Monospace
                || *family == FontFamily::Name("terminal-wide".into());
            // Preserve installed fonts and the embedded Latin/emoji metrics.
            names.push(if mono { MONOSPACE } else { PROPORTIONAL }.into());
        }
        select(fonts, Han::Chinese);
    }

    pub(super) fn select(fonts: &mut FontDefinitions, han: Han) {
        // The pinned collection: JP=0, KR=1, SC=2; mono faces start at index 5.
        let index = match han { Han::Chinese => 2, Han::Japanese => 0, Han::Korean => 1 };
        for (name, offset) in [(PROPORTIONAL, 0), (MONOSPACE, 5)] {
            if let Some(data) = fonts.font_data.get_mut(name) {
                // FontData borrows BYTES; changing locale clones only metadata.
                std::sync::Arc::make_mut(data).index = index + offset;
            }
        }
    }
}

/// Loads a face under its own name, or returns that name if an earlier role loaded it.
fn load(
    fonts: &mut FontDefinitions,
    find: &impl Fn(&str) -> Option<PathBuf>,
    face: Face,
) -> Option<String> {
    let name = face.name();
    if fonts.font_data.contains_key(&name) {
        return Some(name);
    }
    let bytes = std::fs::read(find(face.file)?).ok()?;
    // egui refuses to start on a face it cannot open, so a collection must hold the face.
    if face.index > 0 && !holds_face(&bytes, face.index) {
        return None;
    }
    let mut data = FontData::from_owned(bytes);
    data.index = face.index;
    if let Some(weight) = face.weight {
        data = data.tweak(FontTweak {
            coords: VariationCoords::new([("wght", weight)]),
            ..Default::default()
        });
    }
    fonts.font_data.insert(name.clone(), data.into());
    Some(name)
}

/// Whether TTC collection bytes hold a face at `index`: the header is "ttcf", a version,
/// then the face count.
fn holds_face(bytes: &[u8], index: u32) -> bool {
    bytes.len() >= 12
        && &bytes[..4] == b"ttcf"
        && u32::from_be_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]) > index
}

/// Font files in the user's and the system's font folders, by file name, so no
/// fontconfig library is needed. Earlier folders win: the user's own come first.
#[cfg(not(windows))]
mod system {
    use std::{collections::HashMap, path::PathBuf};

    /// Folder levels below each root, as in truetype/dejavu/DejaVuSans.ttf.
    const DEPTH: usize = 3;
    /// Directory entries read in all, so an unusual font tree cannot stall startup.
    const ENTRIES: usize = 50_000;

    pub(super) fn installed() -> HashMap<String, PathBuf> {
        let home = std::env::home_dir().filter(|home| home.is_absolute());
        let absolute = |name: &str| std::env::var_os(name).map(PathBuf::from).filter(|p| p.is_absolute());
        let mut roots = Vec::new();
        if let Some(data) = absolute("XDG_DATA_HOME").or_else(|| home.as_ref().map(|h| h.join(".local/share"))) {
            roots.push(data.join("fonts"));
        }
        if let Some(home) = &home {
            roots.push(home.join(".fonts"));
        }
        // The XDG Base Directory default when XDG_DATA_DIRS is unset or empty.
        let shared = std::env::var_os("XDG_DATA_DIRS")
            .filter(|dirs| !dirs.is_empty())
            .unwrap_or_else(|| "/usr/local/share:/usr/share".into());
        roots.extend(
            std::env::split_paths(&shared)
                .filter(|dir| dir.is_absolute())
                .map(|dir| dir.join("fonts")),
        );
        index(&roots, DEPTH, ENTRIES)
    }

    /// Font files below `roots`, at most `depth` folders down, reading at most
    /// `entries` directory entries. Linked folders are not followed.
    pub(super) fn index(roots: &[PathBuf], depth: usize, mut entries: usize) -> HashMap<String, PathBuf> {
        let mut found = HashMap::new();
        for root in roots {
            let mut folders = vec![(root.clone(), 0)];
            while let Some((folder, level)) = folders.pop() {
                let Ok(items) = std::fs::read_dir(&folder) else { continue };
                for item in items.flatten() {
                    if entries == 0 {
                        return found;
                    }
                    entries -= 1;
                    let Ok(kind) = item.file_type() else { continue };
                    let path = item.path();
                    if kind.is_dir() {
                        if level < depth {
                            folders.push((path, level + 1));
                        }
                        continue;
                    }
                    // A linked file counts when it leads to a file.
                    if !(kind.is_file() || (kind.is_symlink() && path.is_file())) {
                        continue;
                    }
                    let Some(name) = item.file_name().to_str().map(str::to_owned) else { continue };
                    let lower = name.to_ascii_lowercase();
                    if [".ttf", ".ttc", ".otf"].iter().any(|ext| lower.ends_with(ext)) {
                        found.entry(name).or_insert(path);
                    }
                }
            }
        }
        found
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(fonts: &FontDefinitions, family: FontFamily) -> Vec<String> {
        fonts.families[&family].clone()
    }

    #[test]
    fn missing_system_fonts_keep_embedded_fonts_and_heading_family() {
        let definitions = build(&PLAN, |_| None);
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

    /// Minimal Linux installs must still draw source text, rather than replacement boxes.
    #[cfg(target_os = "linux")]
    #[test]
    fn cjk_source_and_preview_have_glyphs_without_system_fonts() {
        let ctx = eframe::egui::Context::default();
        ctx.set_fonts(build(&PLAN, |_| None));
        let mut output = ctx.run_ui(Default::default(), |ui| {
            ui.ctx().fonts_mut(|fonts| {
                for family in [
                    FontFamily::Monospace,
                    FontFamily::Proportional,
                    FontFamily::Name("semibold".into()),
                    FontFamily::Name("terminal-wide".into()),
                ] {
                    let id = eframe::egui::FontId::new(13.0, family);
                    // epaint's has_glyph compares face IDs with the replacement
                    // face, which also owns valid glyphs. Check the actual cmap
                    // coverage across this family's fallback chain instead.
                    let mut font = fonts.fonts.font(&id.family);
                    let characters = font.characters();
                    for text in ["简体中文 文件路径", "繁體中文 資料夾", "日本語 ひらがな カタカナ", "한국어 파일 변경"] {
                        for c in text.chars() {
                            assert!(characters.contains_key(&c), "Missing glyph {c:?} in {:?}", id.family);
                        }
                    }
                    assert!(!characters.contains_key(&'\u{0378}'), "an unassigned code point is still missing");
                }
            });
        });
        output.textures_delta.clear();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn cjk_fallback_preserves_ascii_layout_and_shares_font_bytes() {
        use eframe::egui::{Color32, Context, FontId};
        let definitions = build(&PLAN, |_| None);
        let measure = |definitions: FontDefinitions| {
            let ctx = Context::default();
            ctx.set_fonts(definitions);
            let mut sizes = Vec::new();
            let mut output = ctx.run_ui(Default::default(), |ui| {
                ui.ctx().fonts_mut(|fonts| {
                    for id in [FontId::monospace(13.0), FontId::proportional(13.0)] {
                        let galley = fonts.layout_no_wrap("local x = 123; return x + 1".into(), id, Color32::WHITE);
                        sizes.push(galley.size());
                    }
                });
            });
            output.textures_delta.clear();
            sizes
        };
        assert_eq!(measure(definitions.clone()), measure(FontDefinitions::default()));
        for (han, proportional, mono) in [(Han::Chinese, 2, 7), (Han::Japanese, 0, 5), (Han::Korean, 1, 6)] {
            let mut localized = definitions.clone();
            linux_cjk::select(&mut localized, han);
            let p = &localized.font_data[linux_cjk::PROPORTIONAL];
            let m = &localized.font_data[linux_cjk::MONOSPACE];
            assert_eq!((p.index, m.index), (proportional, mono));
            assert_eq!(localized.families, definitions.families, "locale keeps fallback precedence");
            assert!(std::ptr::eq(p.font.as_ptr(), m.font.as_ptr()), "faces share immutable bytes");
            assert!(std::ptr::eq(p.font.as_ptr(), definitions.font_data[linux_cjk::PROPORTIONAL].font.as_ptr()));
        }
        assert_eq!(definitions.font_data[linux_cjk::PROPORTIONAL].index, 2, "changing a locale preserves the cached definition");
    }

    /// Bytes that pass for a collection of two faces; past the header nothing is parsed.
    const TWO_FACES: &[u8] = b"ttcf\x00\x01\x00\x00\x00\x00\x00\x02font bytes";

    /// With every candidate present, each role takes its first choice, in role order.
    /// The bytes are never parsed: FontDefinitions only records them.
    #[test]
    fn installed_fonts_fill_each_role_in_order() {
        let file = std::env::temp_dir().join(format!("rocket-font-roles-{}", std::process::id()));
        std::fs::write(&file, TWO_FACES).unwrap();
        let fonts = build(&PLAN, |_| Some(file.clone()));
        std::fs::remove_file(&file).unwrap();
        let name = Face::name;
        let (regular, semibold) = PLAN.interface[0];
        let fallbacks: Vec<String> = PLAN.fallbacks.iter().map(|role| role[0].name()).collect();
        let proportional = names(&fonts, FontFamily::Proportional);
        assert_eq!(proportional[0], name(regular));
        #[cfg(not(target_os = "linux"))]
        let expected_fallbacks = fallbacks.clone();
        #[cfg(target_os = "linux")]
        let expected_fallbacks = [fallbacks.as_slice(), &[linux_cjk::PROPORTIONAL.into()]].concat();
        assert!(proportional.ends_with(&expected_fallbacks));
        let mut interface_then_fallbacks = vec![name(regular)];
        interface_then_fallbacks.extend(fallbacks.iter().cloned());
        #[cfg(target_os = "linux")]
        interface_then_fallbacks.push(linux_cjk::MONOSPACE.into());
        assert!(names(&fonts, FontFamily::Monospace).ends_with(&interface_then_fallbacks));
        let terminal = names(&fonts, FontFamily::Name("terminal-wide".into()));
        assert_eq!(terminal[0], fallbacks[PLAN.emoji]);
        assert_eq!(terminal.iter().filter(|n| **n == fallbacks[PLAN.emoji]).count(), 1);
        let bold = names(&fonts, FontFamily::Name("semibold".into()));
        assert_eq!(bold[0], name(semibold));
        assert_eq!(bold[1..], proportional[..]);
    }

    /// Japanese Han order puts Yu Gothic UI just before Microsoft YaHei in every family,
    /// Korean puts Malgun Gothic there; every other face keeps its place.
    #[cfg(windows)]
    #[test]
    fn han_order_moves_only_the_wanted_face() {
        let file = std::env::temp_dir().join(format!("rocket-font-han-{}", std::process::id()));
        std::fs::write(&file, TWO_FACES).unwrap();
        let chinese_first = build(&PLAN, |_| Some(file.clone()));
        std::fs::remove_file(&file).unwrap();
        let mut japanese = chinese_first.clone();
        han_first(&mut japanese, &PLAN, Han::Japanese);
        assert_eq!(chinese_first.families.len(), japanese.families.len());
        for (family, names) in &chinese_first.families {
            let chinese = names.iter().position(|n| n == "msyh.ttc").unwrap();
            let yu_gothic = names.iter().position(|n| n == "YuGothM.ttc#1").unwrap();
            assert!(chinese < yu_gothic, "{family:?} starts Chinese-first");
            let mut expected = names.clone();
            let moved = expected.remove(yu_gothic);
            expected.insert(chinese, moved);
            assert_eq!(japanese.families[family], expected, "{family:?}");
        }
        let mut again = japanese.clone();
        han_first(&mut again, &PLAN, Han::Japanese);
        assert_eq!(again.families, japanese.families, "already Japanese-first stays so");
        let mut chinese = chinese_first.clone();
        han_first(&mut chinese, &PLAN, Han::Chinese);
        assert_eq!(chinese.families, chinese_first.families, "Chinese is the plan's own order");
        let mut korean = chinese_first.clone();
        han_first(&mut korean, &PLAN, Han::Korean);
        for (family, names) in &chinese_first.families {
            let chinese = names.iter().position(|n| n == "msyh.ttc").unwrap();
            let malgun = names.iter().position(|n| n == "malgun.ttf").unwrap();
            let mut expected = names.clone();
            let moved = expected.remove(malgun);
            expected.insert(chinese, moved);
            assert_eq!(korean.families[family], expected, "{family:?}");
        }
        assert_eq!(chinese_first.font_data["YuGothM.ttc#1"].index, 1, "Yu Gothic UI is the collection's second face");
    }

    /// A collection without the face asked for is passed over for the role's next file:
    /// egui would refuse to start on a face it cannot open.
    #[cfg(windows)]
    #[test]
    fn a_collection_without_the_face_passes_to_the_next_file() {
        let one = std::env::temp_dir().join(format!("rocket-font-one-face-{}", std::process::id()));
        let two = std::env::temp_dir().join(format!("rocket-font-two-faces-{}", std::process::id()));
        std::fs::write(&one, b"ttcf\x00\x01\x00\x00\x00\x00\x00\x01font bytes").unwrap();
        std::fs::write(&two, TWO_FACES).unwrap();
        let fonts = build(&PLAN, |file| Some(if file == "YuGothM.ttc" { one.clone() } else { two.clone() }));
        std::fs::remove_file(&one).unwrap();
        std::fs::remove_file(&two).unwrap();
        assert!(!fonts.font_data.contains_key("YuGothM.ttc#1"));
        assert!(fonts.font_data.contains_key("meiryo.ttc"), "Meiryo takes Japanese");
        assert!(!holds_face(b"font bytes", 1) && !holds_face(b"ttcf", 0), "plain or short bytes hold no second face");
    }

    /// Arabic in Rocket's own fonts, laid out by the vendored epaint's bidirectional
    /// patch (vendor/epaint/patch.md). The cases come from Download Engine's tests of the
    /// same patch. A right-to-left word appears reversed in a row read left to right.
    #[cfg(windows)]
    mod right_to_left {
        use eframe::egui::{self, Color32, FontId, text::{Galley, LayoutJob}};
        use std::sync::Arc;

        fn context() -> egui::Context {
            let ctx = egui::Context::default();
            crate::style::install(&ctx);
            ctx
        }

        fn layout(ctx: &egui::Context, text: &str, wrap_width: f32) -> Arc<Galley> {
            let job = LayoutJob::simple(text.to_owned(), FontId::proportional(13.0), Color32::WHITE, wrap_width);
            let mut galley = None;
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                galley = Some(ui.ctx().fonts_mut(|fonts| fonts.layout_job(job.clone())));
            });
            output.textures_delta.clear();
            galley.unwrap()
        }

        /// Each row's characters from left to right as drawn, leaving out zero-width
        /// glyphs (marks, ligature continuations).
        fn visual_rows(galley: &Galley) -> Vec<String> {
            galley
                .rows
                .iter()
                .map(|row| {
                    let glyphs: Vec<_> = row.row.glyphs.iter().filter(|glyph| glyph.advance_width > 0.0).collect();
                    assert!(glyphs.windows(2).all(|pair| pair[0].pos.x < pair[1].pos.x), "glyphs are stored left to right");
                    glyphs.iter().map(|glyph| glyph.chr).collect()
                })
                .collect()
        }

        /// Words shaped one by one, in right-to-left reading order: what a sentence of
        /// them should look like. A font may join lam and alef into one glyph, so
        /// reversing the characters themselves would not do.
        fn rtl_words(ctx: &egui::Context, text: &str) -> String {
            text.split(' ').rev().map(|word| visual_rows(&layout(ctx, word, f32::INFINITY)).join("")).collect::<Vec<_>>().join(" ")
        }

        fn assert_one_glyph_per_char(galley: &Galley) {
            let glyphs: usize = galley.rows.iter().map(|row| row.row.glyphs.len()).sum();
            assert_eq!(glyphs, galley.job.text.chars().count(), "{:?}", galley.job.text);
        }

        #[test]
        fn arabic_has_glyphs_in_rockets_fonts() {
            let ctx = context();
            let text = "العربية إيقاف التشغيل خلال ثانية، قائمة؟ ٦٠";
            // Fonts exist once a frame has run.
            let mut found = false;
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                found = ui.ctx().fonts_mut(|fonts| fonts.has_glyphs(&FontId::proportional(13.0), text));
            });
            output.textures_delta.clear();
            assert!(found, "Segoe UI or a fallback draws every Arabic letter");
        }

        #[test]
        fn latin_words_and_numbers_keep_reading_order_inside_arabic() {
            let ctx = context();
            let cases = [
                ("إيقاف التشغيل خلال 60 ثانية", format!("{} 60 {}", rtl_words(&ctx, "ثانية"), rtl_words(&ctx, "إيقاف التشغيل خلال"))),
                (
                    "نسخة من Rocket تبدأ مع Windows",
                    format!("Windows {} Rocket {}", rtl_words(&ctx, "تبدأ مع"), rtl_words(&ctx, "نسخة من")),
                ),
                ("اكتمل Windows 62.4%", format!("Windows 62.4% {}", rtl_words(&ctx, "اكتمل"))),
                ("بعد ٦٠ ثانية", format!("{} ٦٠ {}", rtl_words(&ctx, "ثانية"), rtl_words(&ctx, "بعد"))),
                ("دفع main إلى origin", format!("origin {} main {}", rtl_words(&ctx, "إلى"), rtl_words(&ctx, "دفع"))),
            ];
            for (text, expected) in cases {
                let galley = layout(&ctx, text, f32::INFINITY);
                assert_eq!(visual_rows(&galley), [expected], "{text:?}");
                assert_one_glyph_per_char(&galley);
            }
        }

        #[test]
        fn arabic_ligatures_keep_each_source_character() {
            let ctx = context();
            for text in ["لا", "لأ", "الإسبات", "قائمة الانتظار الرئيسية"] {
                let galley = layout(&ctx, text, f32::INFINITY);
                assert_one_glyph_per_char(&galley);
                let mut source: Vec<char> = text.chars().collect();
                let mut drawn: Vec<char> = galley.rows.iter().flat_map(|row| row.row.glyphs.iter().map(|glyph| glyph.chr)).collect();
                source.sort_unstable();
                drawn.sort_unstable();
                assert_eq!(drawn, source, "{text}");
            }
        }

        #[test]
        fn wrapped_arabic_rows_read_top_to_bottom() {
            let ctx = context();
            let text = "تعذر دفع الفرع لأن الفرع البعيد يحتوي على إيداعات ليست لديك. اجلب أولًا ثم أعد المحاولة باستخدام git pull.";
            let rows = visual_rows(&layout(&ctx, text, 160.0));
            assert!(rows.len() >= 3, "{rows:?}");
            // The first row starts the sentence at its right end; the last row ends it.
            assert!(rows[0].trim_end().ends_with(&rtl_words(&ctx, "تعذر")), "{rows:?}");
            assert!(rows.last().unwrap().starts_with(".git pull"), "{rows:?}");
        }

        /// The other languages' text needs no right-to-left layout and keeps its order.
        #[test]
        fn left_to_right_text_is_unchanged() {
            let ctx = context();
            for text in ["Commit & Push 3 staged files", "Изменения · 2 файла", "変更 2 個のファイル", "변경 사항 2개"] {
                let rows = visual_rows(&layout(&ctx, text, f32::INFINITY));
                assert_eq!(rows, [text.replace(['\u{200B}'], "")], "{text:?}");
            }
        }
    }

    /// DejaVu Sans alone fills the interface and symbol roles once, with its own bold.
    #[cfg(not(windows))]
    #[test]
    fn a_later_family_brings_its_own_semibold_without_duplicates() {
        let file = std::env::temp_dir().join(format!("rocket-font-dejavu-{}", std::process::id()));
        std::fs::write(&file, b"font bytes").unwrap();
        let fonts = build(&PLAN, |name| name.starts_with("DejaVuSans").then(|| file.clone()));
        std::fs::remove_file(&file).unwrap();
        let proportional = names(&fonts, FontFamily::Proportional);
        assert_eq!(proportional[0], "DejaVuSans.ttf");
        assert_eq!(proportional.iter().filter(|n| *n == "DejaVuSans.ttf").count(), 1);
        let monospace = names(&fonts, FontFamily::Monospace);
        assert_eq!(monospace.iter().filter(|n| *n == "DejaVuSans.ttf").count(), 1);
        assert_eq!(names(&fonts, FontFamily::Name("semibold".into()))[0], "DejaVuSans-Bold.ttf");
    }

    #[cfg(not(windows))]
    #[test]
    fn font_folders_are_searched_by_name_within_their_bounds() {
        let root = std::env::temp_dir().join(format!("rocket-font-index-{}", std::process::id()));
        let user = root.join("user");
        let shared = root.join("shared");
        for folder in [user.clone(), shared.join("truetype/dejavu"), shared.join("a/b/c/d")] {
            std::fs::create_dir_all(folder).unwrap();
        }
        for file in [
            user.join("DejaVuSans.ttf"),
            shared.join("truetype/dejavu/DejaVuSans.ttf"),
            shared.join("truetype/dejavu/DejaVuSans-Bold.TTF"),
            shared.join("truetype/dejavu/README"),
            shared.join("a/b/c/d/Deep.ttf"),
        ] {
            std::fs::write(file, b"font bytes").unwrap();
        }
        std::os::unix::fs::symlink(shared.join("truetype"), user.join("linked")).unwrap();
        let found = system::index(&[user.clone(), shared.clone()], 3, 1_000);
        assert_eq!(found["DejaVuSans.ttf"], user.join("DejaVuSans.ttf"), "the user's folder wins");
        assert_eq!(found["DejaVuSans-Bold.TTF"], shared.join("truetype/dejavu/DejaVuSans-Bold.TTF"));
        assert!(!found.contains_key("README"));
        assert!(!found.contains_key("Deep.ttf"), "four folders down is past the depth bound");
        assert!(found.values().all(|path| !path.starts_with(user.join("linked"))));
        assert!(system::index(&[shared.clone()], 3, 1).len() <= 1, "the entry budget stops the walk");
        std::fs::remove_dir_all(root).unwrap();
    }
}
