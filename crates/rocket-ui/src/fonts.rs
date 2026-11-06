//! Local system fonts supplement egui's embedded fonts. Nothing is downloaded.
use eframe::egui::{FontData, FontDefinitions, FontFamily, FontTweak, epaint::text::VariationCoords};
use std::{path::PathBuf, sync::OnceLock};

/// The fonts in the order the interface language reads Han characters: the same
/// character takes different forms in Chinese and Japanese fonts (直, 骨, 角).
pub(crate) fn definitions() -> FontDefinitions {
    let mut fonts = loaded();
    if crate::i18n::japanese_han() {
        japanese_first(&mut fonts, &PLAN);
    }
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
    /// The Chinese and the Japanese fallback, which [`japanese_first`] swaps; None when
    /// one collection covers both.
    han: Option<(usize, usize)>,
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
    han: Some((0, 1)),
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

/// Moves the loaded Japanese face before the Chinese one in every family, so Han
/// characters take their Japanese forms. Both stay loaded; only the order changes.
fn japanese_first(fonts: &mut FontDefinitions, plan: &Plan) {
    let Some((chinese, japanese)) = plan.han else { return };
    let loaded = |role: usize| plan.fallbacks[role].iter().map(|f| f.name()).find(|name| fonts.font_data.contains_key(name));
    let (Some(chinese), Some(japanese)) = (loaded(chinese), loaded(japanese)) else { return };
    for family in fonts.families.values_mut() {
        let at = |name: &str| family.iter().position(|n| n == name);
        if let (Some(c), Some(j)) = (at(&chinese), at(&japanese))
            && j > c
        {
            let name = family.remove(j);
            family.insert(c, name);
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
    fonts
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
        assert!(proportional.ends_with(&fallbacks));
        let mut interface_then_fallbacks = vec![name(regular)];
        interface_then_fallbacks.extend(fallbacks.iter().cloned());
        assert!(names(&fonts, FontFamily::Monospace).ends_with(&interface_then_fallbacks));
        let terminal = names(&fonts, FontFamily::Name("terminal-wide".into()));
        assert_eq!(terminal[0], fallbacks[PLAN.emoji]);
        assert_eq!(terminal.iter().filter(|n| **n == fallbacks[PLAN.emoji]).count(), 1);
        let bold = names(&fonts, FontFamily::Name("semibold".into()));
        assert_eq!(bold[0], name(semibold));
        assert_eq!(bold[1..], proportional[..]);
    }

    /// Japanese Han order puts Yu Gothic UI just before Microsoft YaHei in every family;
    /// every other face keeps its place.
    #[cfg(windows)]
    #[test]
    fn japanese_han_moves_only_the_japanese_face() {
        let file = std::env::temp_dir().join(format!("rocket-font-han-{}", std::process::id()));
        std::fs::write(&file, TWO_FACES).unwrap();
        let chinese_first = build(&PLAN, |_| Some(file.clone()));
        std::fs::remove_file(&file).unwrap();
        let mut japanese = chinese_first.clone();
        japanese_first(&mut japanese, &PLAN);
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
        japanese_first(&mut again, &PLAN);
        assert_eq!(again.families, japanese.families, "already Japanese-first stays so");
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
