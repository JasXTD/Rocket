//! Local system fonts supplement egui's embedded fonts. Nothing is downloaded.
use eframe::egui::{FontData, FontDefinitions, FontFamily, FontTweak, epaint::text::VariationCoords};
use std::{path::PathBuf, sync::OnceLock};

pub(crate) fn definitions() -> FontDefinitions {
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

/// A font file and, for a variable font, the weight to draw it at.
#[derive(Clone, Copy)]
struct Face {
    file: &'static str,
    weight: Option<f32>,
}
const fn face(file: &'static str) -> Face {
    Face { file, weight: None }
}

/// One platform's font files for each role, each list in order of preference.
struct Plan {
    /// Interface faces, each with the semibold of its own family.
    interface: &'static [(Face, Face)],
    /// One installed family per writing system or symbol set, rather than every font.
    /// TTC collections use their first face (FontData's default index, 0).
    fallbacks: &'static [&'static [&'static str]],
    /// The fallback that shapes whole emoji sequences in the terminal.
    emoji: usize,
}

#[cfg(windows)]
const PLAN: Plan = Plan {
    interface: &[(face("segoeui.ttf"), face("seguisb.ttf"))],
    fallbacks: &[
        &["msyh.ttc", "msjh.ttc", "simsun.ttc"],
        &["YuGothM.ttc", "meiryo.ttc", "msgothic.ttc"],
        &["malgun.ttf"],
        &["seguiemj.ttf"],
        &["seguisym.ttf"],
    ],
    emoji: 3,
};

/// Ubuntu Sans ships as one variable file; its weight axis gives both faces.
#[cfg(not(windows))]
const UBUNTU_SANS: &str = "UbuntuSans[wdth,wght].ttf";
#[cfg(not(windows))]
const PLAN: Plan = Plan {
    interface: &[
        (
            Face { file: UBUNTU_SANS, weight: Some(400.0) },
            Face { file: UBUNTU_SANS, weight: Some(600.0) },
        ),
        (face("NotoSans-Regular.ttf"), face("NotoSans-SemiBold.ttf")),
        (face("Ubuntu-R.ttf"), face("Ubuntu-M.ttf")),
        (face("DejaVuSans.ttf"), face("DejaVuSans-Bold.ttf")),
        (face("LiberationSans-Regular.ttf"), face("LiberationSans-Bold.ttf")),
    ],
    fallbacks: &[
        // One collection covers Chinese, Japanese and Korean; its first face is Japanese.
        &["NotoSansCJK-Regular.ttc", "DroidSansFallbackFull.ttf", "wqy-microhei.ttc", "wqy-zenhei.ttc"],
        // Outline emoji only: colour-bitmap fonts such as Noto Color Emoji would draw blanks.
        &["NotoEmoji-Regular.ttf", "Symbola.ttf", "Symbola_hint.ttf"],
        &["NotoSansSymbols2-Regular.ttf", "DejaVuSans.ttf"],
    ],
    emoji: 1,
};

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
            .find_map(|file| load(&mut fonts, &find, face(*file)))
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
    let name = match face.weight {
        Some(weight) => format!("{} {weight}", face.file),
        None => face.file.to_owned(),
    };
    if fonts.font_data.contains_key(&name) {
        return Some(name);
    }
    let bytes = std::fs::read(find(face.file)?).ok()?;
    let mut data = FontData::from_owned(bytes);
    if let Some(weight) = face.weight {
        data = data.tweak(FontTweak {
            coords: VariationCoords::new([("wght", weight)]),
            ..Default::default()
        });
    }
    fonts.font_data.insert(name.clone(), data.into());
    Some(name)
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

    /// With every candidate present, each role takes its first choice, in role order.
    /// The bytes are never parsed: FontDefinitions only records them.
    #[test]
    fn installed_fonts_fill_each_role_in_order() {
        let file = std::env::temp_dir().join(format!("rocket-font-roles-{}", std::process::id()));
        std::fs::write(&file, b"font bytes").unwrap();
        let fonts = build(&PLAN, |_| Some(file.clone()));
        std::fs::remove_file(&file).unwrap();
        let name = |face: Face| match face.weight {
            Some(weight) => format!("{} {weight}", face.file),
            None => face.file.to_owned(),
        };
        let (regular, semibold) = PLAN.interface[0];
        let fallbacks: Vec<String> = PLAN.fallbacks.iter().map(|role| role[0].to_owned()).collect();
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
