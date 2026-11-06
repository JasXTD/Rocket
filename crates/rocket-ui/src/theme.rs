//! Themes: the six choices in Preferences → Appearance and the palette each one
//! shows. Clay, the default, is Claude Code's colours, following Windows or kept dark
//! or light; Ocean is Rocket's original navy. Custom starts from Ocean, Dark, Light or
//! a Clay palette, takes its own accent and can show a picture behind the main
//! window's panes (see [`picture`]).
use crate::style::Palette;
use eframe::egui::{self, Color32};
use std::ops::RangeInclusive;

mod appearance;
pub mod picture;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Choice {
    /// The default: Claude Code's colours, in the mode [`Settings::clay`] picks.
    #[default]
    Clay,
    /// Rocket's original navy and cyan, the default before Clay.
    Ocean,
    Dark,
    Light,
    /// Light or Dark, following Windows as it changes.
    System,
    Custom,
}

impl Choice {
    /// In the order Preferences, View → Appearance and the palette show them.
    pub const ALL: [Self; 6] = [Self::Clay, Self::Ocean, Self::Dark, Self::Light, Self::System, Self::Custom];

    pub fn label(self) -> &'static str {
        match self {
            Self::Ocean => "Ocean",
            Self::Dark => "Dark",
            Self::Light => "Light",
            Self::System => "System",
            Self::Clay => "Clay",
            Self::Custom => "Custom",
        }
    }

    /// The key used in automation ids and the command palette.
    pub fn key(self) -> &'static str {
        match self {
            Self::Ocean => "ocean",
            Self::Dark => "dark",
            Self::Light => "light",
            Self::System => "system",
            Self::Clay => "clay",
            Self::Custom => "custom",
        }
    }

    /// The theme record's number for this choice: the numbers older builds wrote, where
    /// 0 was the navy Default (now Ocean). Clay came after Custom; [`Choice::ALL`] is
    /// only the order shown.
    fn code(self) -> u8 {
        match self {
            Self::Ocean => 0,
            Self::Dark => 1,
            Self::Light => 2,
            Self::System => 3,
            Self::Custom => 4,
            Self::Clay => 5,
        }
    }

    fn from_code(code: u8) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.code() == code)
    }
}

/// Clay's mode: following Windows, as Claude's app does, or always dark or light.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ClayMode {
    #[default]
    FollowWindows,
    Dark,
    Light,
}

impl ClayMode {
    pub const ALL: [Self; 3] = [Self::FollowWindows, Self::Dark, Self::Light];

    pub fn label(self) -> &'static str {
        match self {
            Self::FollowWindows => "Follow Windows",
            Self::Dark => "Dark",
            Self::Light => "Light",
        }
    }

    /// The key used in automation ids and state.
    pub fn key(self) -> &'static str {
        match self {
            Self::FollowWindows => "follow",
            Self::Dark => "dark",
            Self::Light => "light",
        }
    }
}

/// The palette a Custom theme starts from; Ocean by default, as its pictures were
/// made over it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Base {
    #[default]
    Ocean,
    Dark,
    Light,
    ClayDark,
    ClayLight,
}

impl Base {
    /// Also the theme record's numbers: new bases go at the end.
    pub const ALL: [Self; 5] = [Self::Ocean, Self::Dark, Self::Light, Self::ClayDark, Self::ClayLight];

    pub fn label(self) -> &'static str {
        match self {
            Self::Ocean => "Ocean",
            Self::Dark => "Dark",
            Self::Light => "Light",
            Self::ClayDark => "Clay dark",
            Self::ClayLight => "Clay light",
        }
    }

    /// The key used in automation ids and state.
    pub fn key(self) -> &'static str {
        match self {
            Self::Ocean => "ocean",
            Self::Dark => "dark",
            Self::Light => "light",
            Self::ClayDark => "clay-dark",
            Self::ClayLight => "clay-light",
        }
    }

    pub fn palette(self) -> Palette {
        match self {
            Self::Ocean => Palette::NAVY,
            Self::Dark => Palette::GRAPHITE,
            Self::Light => Palette::LIGHT,
            Self::ClayDark => Palette::CLAY_DARK,
            Self::ClayLight => Palette::CLAY_LIGHT,
        }
    }

    pub fn light(self) -> bool {
        matches!(self, Self::Light | Self::ClayLight)
    }

    fn clay(self) -> bool {
        matches!(self, Self::ClayDark | Self::ClayLight)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Accent {
    /// The picture's most common vivid colour; the base palette's accent without one.
    FromPicture,
    Colour([u8; 3]),
}

/// The accent swatches offered in Preferences.
pub const SWATCHES: [(&str, [u8; 3]); 8] = [
    ("Cyan", [34, 195, 238]),
    ("Blue", [91, 141, 239]),
    ("Violet", [167, 139, 250]),
    ("Pink", [244, 114, 182]),
    ("Coral", [248, 113, 135]),
    ("Orange", [255, 156, 66]),
    ("Amber", [242, 179, 91]),
    ("Green", [52, 211, 153]),
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Picture {
    None,
    Astronaut,
    SpaceCat,
    Planet,
    Stars,
    /// The user's picture, copied into Rocket's storage under this file name.
    Own(String),
}

impl Picture {
    pub const BUILT_IN: [Self; 4] = [Self::Astronaut, Self::SpaceCat, Self::Planet, Self::Stars];

    pub fn label(&self) -> &'static str {
        match self {
            Self::None => "None",
            Self::Astronaut => "Astronaut",
            Self::SpaceCat => "Space cat",
            Self::Planet => "Planet",
            Self::Stars => "Stars",
            Self::Own(_) => "Your picture",
        }
    }

    pub fn key(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Astronaut => "astronaut",
            Self::SpaceCat => "space-cat",
            Self::Planet => "planet",
            Self::Stars => "stars",
            Self::Own(_) => "own",
        }
    }
}

/// How the picture fits the window.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Size {
    /// Covers the window; Position picks the part that stays in view.
    #[default]
    Fill,
    /// Shows the whole picture, its inner edge fading into the theme.
    Fit,
}

/// Which side of the picture stays in view.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Position {
    Left,
    Centre,
    #[default]
    Right,
}

/// Percent of the picture that shows through the panes.
pub const VISIBILITY: RangeInclusive<u8> = 10..=60;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Custom {
    pub base: Base,
    pub accent: Accent,
    pub picture: Picture,
    pub size: Size,
    pub position: Position,
    pub visibility: u8,
}

impl Default for Custom {
    fn default() -> Self {
        Self {
            base: Base::Ocean,
            accent: Accent::FromPicture,
            picture: Picture::Astronaut,
            size: Size::Fill,
            position: Position::Right,
            visibility: 30,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Settings {
    pub choice: Choice,
    /// Clay's mode; kept while another theme is chosen.
    pub clay: ClayMode,
    /// Kept while another theme is chosen, so switching back restores it.
    pub custom: Custom,
}

impl Settings {
    /// The Custom theme's settings while it is the chosen theme.
    pub fn custom(&self) -> Option<&Custom> {
        (self.choice == Choice::Custom).then_some(&self.custom)
    }

    /// The state file's theme record. Version 2 adds Clay's mode after version 1's
    /// fields and allows the Clay choice and bases. Older builds refuse it, and with it
    /// the state file, so a record that needs nothing of Clay stays version 1.
    pub(crate) fn encode(&self) -> Vec<u8> {
        let c = &self.custom;
        let clay = self.choice == Choice::Clay || c.base.clay() || self.clay != ClayMode::default();
        let mut bytes = vec![if clay { 2 } else { 1 }, self.choice.code(), index(Base::ALL, c.base)];
        match c.accent {
            Accent::FromPicture => bytes.extend([0, 0, 0, 0]),
            Accent::Colour([r, g, b]) => bytes.extend([1, r, g, b]),
        }
        let (tag, name) = match &c.picture {
            Picture::None => (0, ""),
            Picture::Astronaut => (1, ""),
            Picture::SpaceCat => (2, ""),
            Picture::Planet => (3, ""),
            Picture::Stars => (4, ""),
            Picture::Own(name) => (5, name.as_str()),
        };
        bytes.extend([tag, name.len() as u8]);
        bytes.extend(name.as_bytes());
        bytes.extend([
            index([Size::Fill, Size::Fit], c.size),
            index([Position::Left, Position::Centre, Position::Right], c.position),
            c.visibility,
        ]);
        if clay {
            bytes.push(index(ClayMode::ALL, self.clay));
        }
        bytes
    }

    pub(crate) fn decode(bytes: &[u8]) -> Option<Self> {
        let mut r = bytes.iter().copied();
        let mut next = || r.next();
        let version = next()?;
        if !matches!(version, 1 | 2) {
            return None;
        }
        let choice = Choice::from_code(next()?)?;
        let base = *Base::ALL.get(usize::from(next()?))?;
        if version == 1 && (choice == Choice::Clay || base.clay()) {
            return None;
        }
        let accent = match (next()?, [next()?, next()?, next()?]) {
            (0, [0, 0, 0]) => Accent::FromPicture,
            (1, rgb) => Accent::Colour(rgb),
            _ => return None,
        };
        let tag = next()?;
        let length = usize::from(next()?);
        let name: Vec<u8> = (0..length).map(|_| next()).collect::<Option<_>>()?;
        let picture = match (tag, name.is_empty()) {
            (0, true) => Picture::None,
            (1, true) => Picture::Astronaut,
            (2, true) => Picture::SpaceCat,
            (3, true) => Picture::Planet,
            (4, true) => Picture::Stars,
            (5, false) => {
                let name = String::from_utf8(name).ok()?;
                picture::own_name(&name).then_some(Picture::Own(name))?
            }
            _ => return None,
        };
        let size = *[Size::Fill, Size::Fit].get(usize::from(next()?))?;
        let position = *[Position::Left, Position::Centre, Position::Right].get(usize::from(next()?))?;
        let visibility = next()?;
        let clay = if version == 2 { *ClayMode::ALL.get(usize::from(next()?))? } else { ClayMode::default() };
        if !VISIBILITY.contains(&visibility) || next().is_some() {
            return None;
        }
        Some(Self {
            choice,
            clay,
            custom: Custom { base, accent, picture, size, position, visibility },
        })
    }
}

fn index<T: PartialEq, const N: usize>(all: [T; N], value: T) -> u8 {
    all.iter().position(|v| *v == value).unwrap_or(0) as u8
}

/// The palette a theme shows, and whether egui's light visuals suit it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Look {
    pub palette: Palette,
    pub light: bool,
}

/// What `settings` shows while Windows is in `system` mode. `picture_accent` is the
/// Custom picture's vivid colour once it has been read.
pub fn resolve(settings: &Settings, system: Option<egui::Theme>, picture_accent: Option<Color32>) -> Look {
    let plain = |base: Base| Look { palette: base.palette(), light: base.light() };
    match settings.choice {
        Choice::Ocean => plain(Base::Ocean),
        Choice::Dark => plain(Base::Dark),
        Choice::Light => plain(Base::Light),
        // Without a reported mode, Dark.
        Choice::System if system == Some(egui::Theme::Light) => plain(Base::Light),
        Choice::System => plain(Base::Dark),
        Choice::Clay => match settings.clay {
            ClayMode::Dark => plain(Base::ClayDark),
            ClayMode::Light => plain(Base::ClayLight),
            // As System: light only when Windows says so.
            ClayMode::FollowWindows if system == Some(egui::Theme::Light) => plain(Base::ClayLight),
            ClayMode::FollowWindows => plain(Base::ClayDark),
        },
        Choice::Custom => {
            let c = &settings.custom;
            let base = c.base.palette();
            let light = c.base.light();
            let accent = match c.accent {
                Accent::Colour([r, g, b]) => Some(Color32::from_rgb(r, g, b)),
                Accent::FromPicture if c.picture != Picture::None => picture_accent,
                Accent::FromPicture => None,
            };
            let mut palette = accent.map_or(base, |accent| with_accent(&base, light, accent));
            if c.picture != Picture::None {
                see_through(&mut palette, &base, f32::from(c.visibility) / 100.0);
            }
            Look { palette, light }
        }
    }
}

/// `base` with `accent` and the colours made from it. The accent is lightened or
/// darkened until it reads as text on the base surfaces.
pub fn with_accent(base: &Palette, light: bool, accent: Color32) -> Palette {
    let accent = readable(accent, base.bg, light);
    let dark_ink = mix(accent, Color32::BLACK, 0.82);
    let ink = if contrast(dark_ink, accent) >= contrast(Color32::WHITE, accent) {
        dark_ink
    } else {
        Color32::WHITE
    };
    let towards = if light { Color32::BLACK } else { Color32::WHITE };
    let [r, g, b, _] = accent.to_array();
    let soft = |c: u8| (u16::from(c) * 33 / 255) as u8;
    Palette {
        accent,
        accent_ink: ink,
        accent_soft: Color32::from_rgba_premultiplied(soft(r), soft(g), soft(b), 33),
        selected: if light { mix(Color32::WHITE, accent, 0.14) } else { mix(base.bg, accent, 0.2) },
        accent_text: mix(accent, towards, if light { 0.2 } else { 0.35 }),
        accent_hover: mix(accent, towards, if light { 0.15 } else { 0.25 }),
        accent_light: mix(accent, Color32::WHITE, if light { 0.25 } else { 0.3 }),
        edge_accent: mix(base.window_edge, accent, 0.45),
        ramp_1: mix(base.chip, accent, 0.4),
        ramp_2: mix(base.chip, accent, 0.6),
        ramp_3: mix(base.chip, accent, 0.8),
        menu_hot: mix(base.window, accent, if light { 0.12 } else { 0.18 }),
        ..*base
    }
}

/// Makes the main window's panes let `visibility` (0–1) of the picture through.
fn see_through(palette: &mut Palette, base: &Palette, visibility: f32) {
    let translucent = |color: Color32, opacity: f32| {
        let [r, g, b, _] = color.to_array();
        Color32::from_rgba_unmultiplied(r, g, b, (opacity * 255.0).round() as u8)
    };
    palette.pane_bg = translucent(base.bg, 1.0 - visibility);
    palette.pane_panel = translucent(base.panel, 1.0 - visibility * 0.87);
    palette.pane_deep = translucent(base.deep, 1.0 - visibility * 0.47);
    palette.view_bg = Color32::TRANSPARENT;
    palette.view_panel = translucent(base.panel, 0.13);
    palette.view_raised = translucent(base.raised, 0.4);
    // Diff rows tint the picture instead of covering it.
    palette.added_row = base.green.gamma_multiply(0.16);
    palette.removed_row = base.red.gamma_multiply(0.16);
}

/// Lightens (dark themes) or darkens (light themes) `color` until it has 4.5:1
/// contrast with `surface`.
fn readable(color: Color32, surface: Color32, light: bool) -> Color32 {
    let target = if light { Color32::BLACK } else { Color32::WHITE };
    (0..=20)
        .map(|step| mix(color, target, step as f32 * 0.05))
        .find(|c| contrast(*c, surface) >= 4.5)
        .unwrap_or(target)
}

pub(crate) fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    a.lerp_to_gamma(b, t)
}

/// One sRGB channel in linear light.
pub(crate) fn linear(channel: f32) -> f32 {
    if channel <= 0.04045 { channel / 12.92 } else { ((channel + 0.055) / 1.055).powf(2.4) }
}

/// WCAG relative luminance of an opaque colour.
pub(crate) fn luminance(color: Color32) -> f32 {
    let [r, g, b, _] = color.to_array().map(|c| linear(f32::from(c) / 255.0));
    0.2126 * r + 0.7152 * g + 0.0722 * b
}

/// WCAG contrast ratio between two opaque colours.
pub(crate) fn contrast(a: Color32, b: Color32) -> f32 {
    let (a, b) = (luminance(a), luminance(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn looks() -> Vec<(&'static str, Look)> {
        let mut looks = vec![
            ("Clay, the default", resolve(&Settings::default(), None, None)),
            ("Ocean", resolve(&Settings { choice: Choice::Ocean, ..Default::default() }, None, None)),
            ("Dark", resolve(&Settings { choice: Choice::Dark, ..Default::default() }, None, None)),
            ("Light", resolve(&Settings { choice: Choice::Light, ..Default::default() }, None, None)),
            ("Clay dark", resolve(&Settings { choice: Choice::Clay, clay: ClayMode::Dark, ..Default::default() }, None, None)),
            ("Clay light", resolve(&Settings { choice: Choice::Clay, clay: ClayMode::Light, ..Default::default() }, None, None)),
        ];
        for base in Base::ALL {
            for (name, rgb) in SWATCHES {
                let settings = Settings {
                    choice: Choice::Custom,
                    custom: Custom { base, accent: Accent::Colour(rgb), picture: Picture::None, ..Default::default() },
                    ..Default::default()
                };
                looks.push((name, resolve(&settings, None, None)));
            }
        }
        looks
    }

    #[test]
    fn every_theme_keeps_text_and_accent_readable() {
        for (name, look) in looks() {
            let p = look.palette;
            for (role, color, surfaces, ratio) in [
                ("text", p.text, [p.bg, p.panel, p.raised, p.window, p.card], 4.5),
                ("muted", p.muted, [p.bg, p.panel, p.raised, p.window, p.card], 4.5),
                ("hint", p.hint, [p.bg, p.panel, p.raised, p.window, p.card], 4.5),
                ("accent", p.accent, [p.bg, p.panel, p.bg, p.panel, p.bg], 4.5),
                ("faint", p.faint, [p.bg, p.panel, p.bg, p.panel, p.bg], 3.0),
                ("ink", p.accent_ink, [p.accent; 5], 4.5),
            ] {
                for surface in surfaces {
                    let c = contrast(color, surface);
                    assert!(c >= ratio, "{name}: {role} {color:?} on {surface:?} is {c:.2}:1");
                }
            }
            for (role, ink, fill) in [("amber", p.amber_ink, p.amber), ("green", p.green_ink, p.green), ("red", p.red_ink, p.red)] {
                assert!(contrast(ink, fill) >= 4.5, "{name}: text on {role} is {:.2}:1", contrast(ink, fill));
            }
        }
    }

    #[test]
    fn default_is_clay_ocean_is_the_navy_and_system_follows_windows() {
        let default = Settings::default();
        assert_eq!((default.choice, default.clay), (Choice::Clay, ClayMode::FollowWindows));
        assert_eq!(resolve(&default, None, None).palette, Palette::CLAY_DARK);
        assert_eq!(resolve(&default, Some(egui::Theme::Light), None).palette, Palette::CLAY_LIGHT);
        let ocean = Settings { choice: Choice::Ocean, ..Default::default() };
        assert_eq!(resolve(&ocean, Some(egui::Theme::Light), None).palette, Palette::NAVY, "Ocean stays navy");
        assert_eq!(Choice::ALL[0], Choice::Clay, "the default comes first");
        let system = Settings { choice: Choice::System, ..Default::default() };
        assert_eq!(resolve(&system, Some(egui::Theme::Light), None).palette, Palette::LIGHT);
        assert_eq!(resolve(&system, Some(egui::Theme::Dark), None).palette, Palette::GRAPHITE);
        assert_eq!(resolve(&system, None, None).palette, Palette::GRAPHITE);
    }

    #[test]
    fn clay_follows_windows_unless_its_mode_keeps_it_dark_or_light() {
        let clay = |clay| Settings { choice: Choice::Clay, clay, ..Default::default() };
        let follow = clay(ClayMode::FollowWindows);
        assert_eq!(follow, clay(ClayMode::default()), "Follow Windows is the default");
        assert_eq!(resolve(&follow, Some(egui::Theme::Light), None).palette, Palette::CLAY_LIGHT);
        assert_eq!(resolve(&follow, Some(egui::Theme::Dark), None).palette, Palette::CLAY_DARK);
        assert_eq!(resolve(&follow, None, None).palette, Palette::CLAY_DARK, "no reported mode: dark");
        for system in [Some(egui::Theme::Light), Some(egui::Theme::Dark), None] {
            assert_eq!(resolve(&clay(ClayMode::Dark), system, None), Look { palette: Palette::CLAY_DARK, light: false });
            assert_eq!(resolve(&clay(ClayMode::Light), system, None), Look { palette: Palette::CLAY_LIGHT, light: true });
        }
        // Claude Code's colours: coral on charcoal; terracotta on ivory, deepened to read.
        assert_eq!(Palette::CLAY_DARK.accent, Color32::from_rgb(217, 119, 87));
        assert_eq!(Palette::CLAY_DARK.bg, Color32::from_rgb(31, 30, 29));
        assert_eq!(Palette::CLAY_LIGHT.bg, Color32::from_rgb(245, 244, 237));
        // Custom can start from either mode; its accent replaces the coral.
        let custom = Settings { choice: Choice::Custom, custom: Custom { base: Base::ClayLight, accent: Accent::Colour([52, 211, 153]), picture: Picture::None, ..Default::default() }, ..Default::default() };
        let look = resolve(&custom, None, None);
        assert!(look.light && look.palette.bg == Palette::CLAY_LIGHT.bg && look.palette.accent != Palette::CLAY_LIGHT.accent);
    }

    #[test]
    fn a_record_without_clay_stays_version_1_so_older_builds_keep_reading_it() {
        // Clay is shown first, but each choice keeps the number older builds wrote:
        // Ocean is the navy they called Default.
        let custom = Settings { choice: Choice::Custom, ..Default::default() };
        assert_eq!(custom.encode()[..2], [1, 4]);
        assert_eq!(Settings { choice: Choice::System, ..Default::default() }.encode()[..2], [1, 3]);
        assert_eq!(Settings { choice: Choice::Ocean, ..Default::default() }.encode()[..2], [1, 0]);
        // Anything of Clay's needs version 2, which adds its mode at the end.
        let chosen = Settings { choice: Choice::Clay, ..Default::default() };
        let mode = Settings { choice: Choice::Ocean, clay: ClayMode::Light, ..Default::default() };
        let base = Settings { custom: Custom { base: Base::ClayDark, ..Default::default() }, ..custom.clone() };
        for settings in [chosen, mode, base] {
            let bytes = settings.encode();
            assert_eq!(bytes[0], 2, "{settings:?}");
            assert_eq!(Settings::decode(&bytes), Some(settings));
        }
        let mut old = Settings { choice: Choice::Clay, ..Default::default() }.encode();
        old[0] = 1;
        old.pop();
        assert_eq!(Settings::decode(&old), None, "version 1 has no Clay");
        let mut unknown = Settings { choice: Choice::Clay, clay: ClayMode::Dark, ..Default::default() }.encode();
        *unknown.last_mut().unwrap() = 3;
        assert_eq!(Settings::decode(&unknown), None, "no fourth mode");
    }

    #[test]
    fn a_picture_turns_only_the_main_panes_see_through() {
        let pink = Color32::from_rgb(244, 114, 182);
        let settings = Settings { choice: Choice::Custom, custom: Custom::default(), ..Default::default() };
        let look = resolve(&settings, None, Some(pink));
        let p = look.palette;
        assert_eq!(p.accent, pink, "From picture uses the picture's colour");
        assert_eq!(p.pane_bg.a(), 179, "30% visibility leaves the content area 70% opaque");
        assert!(p.pane_panel.a() > p.pane_bg.a() && p.pane_deep.a() > p.pane_panel.a());
        assert_eq!(p.view_bg, Color32::TRANSPARENT);
        assert_eq!((p.bg, p.window, p.card), (Palette::NAVY.bg, Palette::NAVY.window, Palette::NAVY.card), "dialogs stay solid");
        let none = Settings { custom: Custom { picture: Picture::None, ..Custom::default() }, ..settings };
        let p = resolve(&none, None, Some(pink)).palette;
        assert_eq!((p.accent, p.pane_bg), (Palette::NAVY.accent, Palette::NAVY.bg), "no picture: no picture colour, solid panes");
    }

    #[test]
    fn settings_roundtrip_and_invalid_records_are_refused() {
        let own = Settings {
            choice: Choice::Custom,
            custom: Custom {
                base: Base::Light,
                accent: Accent::Colour([1, 2, 3]),
                picture: Picture::Own("picture-0123456789abcdef.png".into()),
                size: Size::Fit,
                position: Position::Centre,
                visibility: 45,
            },
            ..Default::default()
        };
        for settings in [Settings::default(), own.clone(), Settings { choice: Choice::System, ..Default::default() }] {
            assert_eq!(Settings::decode(&settings.encode()), Some(settings));
        }
        let bytes = own.encode();
        assert_eq!(Settings::decode(&bytes[..bytes.len() - 1]), None, "truncated");
        assert_eq!(Settings::decode(&[bytes.clone(), vec![0]].concat()), None, "trailing byte");
        let mut far = bytes.clone();
        *far.last_mut().unwrap() = 61;
        assert_eq!(Settings::decode(&far), None, "visibility out of range");
        let mut path = Settings { custom: Custom { picture: Picture::Own("..\\x.png".into()), ..own.custom.clone() }, ..own.clone() }.encode();
        assert_eq!(Settings::decode(&path), None, "only Rocket's own picture names load");
        path[1] = 9;
        assert_eq!(Settings::decode(&path), None);
    }
}
