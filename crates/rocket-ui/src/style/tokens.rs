//! Every colour a theme defines, one row per role: Ocean (Rocket's original navy), Dark,
//! Light, then Clay in its dark and light modes (Claude Code's colours: warm
//! charcoal, cream and coral; ivory and terracotta). The accessor functions keep the
//! constant-style names these colours had before themes (`TEXT()` was `TEXT`), so
//! each use reads the theme that is showing.
//! Values live in atomics: a read is one relaxed load, from any thread.
use eframe::egui::Color32;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering::Relaxed};

const fn rgb(r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgb(r, g, b)
}
const fn pack(color: Color32) -> u32 {
    u32::from_le_bytes(color.to_array())
}
const fn unpack(bits: u32) -> Color32 {
    let [r, g, b, a] = bits.to_le_bytes();
    Color32::from_rgba_premultiplied(r, g, b, a)
}

macro_rules! palette {
    ($($(#[$doc:meta])* $name:ident $field:ident: $navy:expr, $dark:expr, $light:expr, $clay:expr, $clay_light:expr;)*) => {
        /// One colour for every role.
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub struct Palette {
            $($(#[$doc])* pub $field: Color32,)*
        }
        impl Palette {
            /// Ocean: Rocket's original navy and cyan, the colours before themes and the
            /// default until Clay.
            pub const NAVY: Self = Self { $($field: $navy,)* };
            /// The Dark theme: neutral graphite.
            pub const GRAPHITE: Self = Self { $($field: $dark,)* };
            pub const LIGHT: Self = Self { $($field: $light,)* };
            /// Clay, dark mode, of the default theme: Claude Code's warm charcoal, cream text
            /// and coral accent.
            pub const CLAY_DARK: Self = Self { $($field: $clay,)* };
            /// Clay, light mode: ivory surfaces, near-black text and a terracotta accent.
            pub const CLAY_LIGHT: Self = Self { $($field: $clay_light,)* };
            fn store(&self) {
                $(ACTIVE[Slot::$name as usize].store(pack(self.$field), Relaxed);)*
            }
            /// The palette that is showing.
            pub fn active() -> Self {
                Self { $($field: $name(),)* }
            }
        }
        #[allow(non_camel_case_types, clippy::upper_case_acronyms)]
        #[derive(Clone, Copy)]
        enum Slot { $($name,)* }
        static ACTIVE: [AtomicU32; [$(stringify!($name),)*].len()] =
            [$(AtomicU32::new(pack($navy)),)*];
        $(
            $(#[$doc])*
            #[allow(non_snake_case)]
            #[inline]
            pub fn $name() -> Color32 {
                unpack(ACTIVE[Slot::$name as usize].load(Relaxed))
            }
        )*
    };
}

const WHITE: Color32 = Color32::WHITE;

palette! {
    // Surfaces: depth comes from fill, not from nested borders.
    /// Menu bar, repository tab strip and status bar.
    DEEP deep: rgb(7, 17, 26), rgb(17, 19, 21), rgb(238, 241, 245), rgb(26, 25, 24), rgb(240, 238, 230);
    BG bg: rgb(10, 22, 33), rgb(22, 24, 27), rgb(248, 250, 252), rgb(31, 30, 29), rgb(245, 244, 237);
    /// Sidebar, toolbar, file lists and pane headers.
    PANEL panel: rgb(13, 27, 40), rgb(27, 30, 34), rgb(255, 255, 255), rgb(38, 38, 36), rgb(250, 249, 245);
    RAISED raised: rgb(20, 37, 54), rgb(35, 39, 44), rgb(243, 246, 249), rgb(48, 48, 46), rgb(240, 238, 230);
    HOVER hover: rgb(22, 41, 59), rgb(41, 46, 52), rgb(237, 241, 246), rgb(53, 52, 49), rgb(234, 232, 223);
    /// Dividers between panes.
    HAIRLINE hairline: rgb(26, 47, 66), rgb(39, 43, 49), rgb(225, 230, 236), rgb(56, 55, 51), rgb(227, 225, 215);
    /// Input and control outlines.
    BORDER border: rgb(36, 62, 86), rgb(56, 62, 70), rgb(201, 210, 220), rgb(74, 73, 68), rgb(209, 206, 194);
    /// The main window's panes: DEEP, BG and PANEL, or see-through versions of them
    /// while a Custom theme's picture shows behind the panes. PANE_BG is the content
    /// area under every view.
    PANE_DEEP pane_deep: rgb(7, 17, 26), rgb(17, 19, 21), rgb(238, 241, 245), rgb(26, 25, 24), rgb(240, 238, 230);
    PANE_BG pane_bg: rgb(10, 22, 33), rgb(22, 24, 27), rgb(248, 250, 252), rgb(31, 30, 29), rgb(245, 244, 237);
    PANE_PANEL pane_panel: rgb(13, 27, 40), rgb(27, 30, 34), rgb(255, 255, 255), rgb(38, 38, 36), rgb(250, 249, 245);
    /// Panels of a view inside the content area: BG and PANEL, or, over a picture,
    /// nothing and a light PANEL wash, so PANE_BG is not stacked twice.
    VIEW_BG view_bg: rgb(10, 22, 33), rgb(22, 24, 27), rgb(248, 250, 252), rgb(31, 30, 29), rgb(245, 244, 237);
    VIEW_PANEL view_panel: rgb(13, 27, 40), rgb(27, 30, 34), rgb(255, 255, 255), rgb(38, 38, 36), rgb(250, 249, 245);
    /// RAISED inside a view, such as a diff's hunk rows; a light wash over a picture.
    VIEW_RAISED view_raised: rgb(20, 37, 54), rgb(35, 39, 44), rgb(243, 246, 249), rgb(48, 48, 46), rgb(240, 238, 230);

    TEXT text: rgb(231, 238, 248), rgb(232, 234, 237), rgb(22, 32, 42), rgb(240, 238, 230), rgb(20, 20, 19);
    MUTED muted: rgb(163, 183, 204), rgb(180, 186, 194), rgb(74, 90, 108), rgb(195, 194, 183), rgb(61, 61, 58);
    FAINT faint: rgb(109, 133, 156), rgb(134, 142, 152), rgb(98, 115, 134), rgb(156, 154, 146), rgb(107, 106, 100);
    /// Secondary text on dialog and card surfaces, where FAINT falls below AA contrast.
    HINT hint: rgb(129, 153, 175), rgb(154, 162, 172), rgb(86, 103, 122), rgb(171, 169, 159), rgb(95, 94, 89);
    /// Quiet text that still reads as content: shortcut keys, paths, code excerpts.
    TEXT_SOFT text_soft: rgb(195, 209, 223), rgb(205, 210, 216), rgb(52, 66, 82), rgb(214, 212, 203), rgb(52, 51, 47);
    /// Entries listed in plain dialogs.
    TEXT_ITEM text_item: rgb(211, 222, 234), rgb(218, 222, 227), rgb(40, 52, 66), rgb(226, 224, 216), rgb(38, 38, 36);

    /// The single accent: selection bars, primary actions and counts.
    BLUE accent: rgb(34, 195, 238), rgb(34, 195, 238), rgb(10, 120, 159), rgb(217, 119, 87), rgb(174, 82, 54);
    /// Text and icons on an accent fill.
    ACCENT_INK accent_ink: rgb(3, 32, 43), rgb(3, 32, 43), WHITE, rgb(31, 30, 29), rgb(255, 255, 255);
    /// 13% accent over the surfaces, premultiplied.
    ACCENT_SOFT accent_soft: Color32::from_rgba_premultiplied(4, 25, 31, 33),
        Color32::from_rgba_premultiplied(4, 25, 31, 33),
        Color32::from_rgba_premultiplied(1, 16, 21, 33),
        Color32::from_rgba_premultiplied(28, 15, 11, 33),
        Color32::from_rgba_premultiplied(22, 10, 6, 33);
    SELECTED selected: rgb(19, 56, 92), rgb(29, 58, 71), rgb(220, 239, 248), rgb(68, 57, 51), rgb(242, 230, 223);
    /// Accent-coloured text, such as a shortcut cell that is recording.
    ACCENT_TEXT accent_text: rgb(127, 221, 245), rgb(127, 221, 245), rgb(8, 98, 130), rgb(235, 162, 132), rgb(162, 77, 48);
    /// A hovered accent button.
    ACCENT_HOVER accent_hover: rgb(86, 213, 244), rgb(86, 213, 244), rgb(8, 104, 138), rgb(227, 139, 109), rgb(154, 71, 48);
    /// A hovered accent bar in a chart.
    ACCENT_LIGHT accent_light: rgb(111, 217, 244), rgb(111, 217, 244), rgb(40, 150, 190), rgb(235, 165, 138), rgb(199, 129, 107);
    /// An outline lit in the accent by the pointer or keyboard focus.
    EDGE_ACCENT edge_accent: rgb(47, 106, 150), rgb(45, 108, 130), rgb(90, 166, 196), rgb(138, 93, 75), rgb(193, 150, 131);
    /// The middle steps of the statistics ramp, which runs from CHIP to the accent.
    RAMP_1 ramp_1: rgb(25, 90, 132), rgb(25, 80, 100), rgb(176, 214, 230), rgb(117, 78, 63), rgb(211, 173, 157);
    RAMP_2 ramp_2: rgb(26, 118, 168), rgb(26, 110, 135), rgb(110, 180, 210), rgb(151, 91, 71), rgb(199, 143, 123);
    RAMP_3 ramp_3: rgb(29, 152, 207), rgb(30, 150, 185), rgb(50, 140, 185), rgb(184, 105, 79), rgb(186, 112, 88);

    GREEN green: rgb(52, 211, 153), rgb(52, 211, 153), rgb(15, 127, 88), rgb(78, 186, 101), rgb(35, 121, 60);
    RED red: rgb(248, 113, 135), rgb(248, 113, 135), rgb(200, 51, 79), rgb(255, 107, 128), rgb(200, 50, 63);
    AMBER amber: rgb(242, 179, 91), rgb(242, 179, 91), rgb(163, 94, 0), rgb(242, 184, 75), rgb(163, 94, 0);
    VIOLET violet: rgb(167, 139, 250), rgb(167, 139, 250), rgb(107, 74, 214), rgb(177, 185, 249), rgb(90, 98, 200);
    /// Graph lanes and activity phases besides the accent and the status colours.
    ORANGE orange: rgb(255, 156, 66), rgb(255, 156, 66), rgb(196, 96, 0), rgb(224, 177, 90), rgb(160, 112, 15);
    PINK pink: rgb(242, 80, 143), rgb(242, 80, 143), rgb(200, 40, 110), rgb(232, 122, 164), rgb(194, 56, 122);
    TEAL teal: rgb(92, 198, 212), rgb(92, 198, 212), rgb(0, 122, 140), rgb(107, 197, 184), rgb(15, 128, 115);
    /// The warning that Secret protection is off.
    WARNING warning: rgb(245, 177, 63), rgb(245, 177, 63), rgb(163, 94, 0), rgb(242, 184, 75), rgb(163, 94, 0);
    GREEN_TEXT green_text: rgb(127, 224, 189), rgb(127, 224, 189), rgb(12, 110, 76), rgb(143, 217, 160), rgb(31, 107, 55);
    VIOLET_TEXT violet_text: rgb(216, 204, 255), rgb(216, 204, 255), rgb(90, 60, 190), rgb(211, 215, 252), rgb(74, 80, 176);
    AMBER_TEXT amber_text: rgb(231, 217, 176), rgb(231, 217, 176), rgb(120, 84, 10), rgb(235, 217, 174), rgb(122, 84, 10);
    /// Text on AMBER, GREEN and RED fills.
    AMBER_INK amber_ink: rgb(58, 42, 0), rgb(58, 42, 0), WHITE, rgb(58, 42, 0), rgb(255, 255, 255);
    WARN_INK warn_ink: rgb(43, 26, 4), rgb(43, 26, 4), WHITE, rgb(43, 26, 4), rgb(255, 255, 255);
    GREEN_INK green_ink: rgb(4, 52, 44), rgb(4, 52, 44), WHITE, rgb(6, 51, 26), rgb(255, 255, 255);
    RED_INK red_ink: rgb(43, 7, 16), rgb(43, 7, 16), WHITE, rgb(58, 10, 18), rgb(255, 255, 255);

    // Dialogs, menus and cards.
    WINDOW window: rgb(15, 32, 49), rgb(28, 31, 35), rgb(244, 246, 249), rgb(42, 41, 39), rgb(250, 249, 245);
    WINDOW_EDGE window_edge: rgb(42, 69, 96), rgb(58, 64, 72), rgb(201, 210, 220), rgb(69, 68, 63), rgb(209, 206, 194);
    /// A hovered control's outline.
    EDGE_HOT edge_hot: rgb(47, 79, 108), rgb(70, 77, 87), rgb(174, 185, 198), rgb(92, 91, 85), rgb(185, 181, 169);
    /// Cards on dialog surfaces, their outline and the dividers between their rows.
    CARD card: rgb(18, 38, 58), rgb(33, 36, 41), WHITE, rgb(48, 48, 46), rgb(255, 255, 255);
    CARD_STROKE card_stroke: rgb(30, 56, 80), rgb(52, 57, 64), rgb(217, 224, 232), rgb(67, 66, 61), rgb(227, 225, 215);
    CARD_LINE card_line: rgb(26, 49, 73), rgb(44, 48, 55), rgb(230, 235, 240), rgb(58, 57, 53), rgb(236, 234, 226);
    /// Side lists inside dialogs: places, page lists, command lists and footers.
    SUNKEN sunken: rgb(12, 26, 40), rgb(24, 26, 30), rgb(238, 241, 245), rgb(35, 35, 33), rgb(240, 238, 230);
    NAV nav: rgb(12, 27, 42), rgb(24, 26, 30), rgb(238, 241, 245), rgb(35, 35, 33), rgb(240, 238, 230);
    FOOTER footer: rgb(13, 29, 46), rgb(25, 28, 32), rgb(238, 241, 245), rgb(37, 37, 35), rgb(240, 238, 230);
    /// Code chips, tag rows and selected entries on dialog surfaces.
    CHIP chip: rgb(19, 40, 60), rgb(36, 40, 46), rgb(234, 238, 243), rgb(51, 50, 47), rgb(236, 234, 226);
    /// Fields on cards.
    FIELD field: rgb(19, 38, 58), rgb(35, 39, 44), WHITE, rgb(44, 43, 41), rgb(255, 255, 255);
    /// A hovered row on a dialog surface.
    LIST_HOT list_hot: rgb(15, 33, 50), rgb(33, 36, 41), rgb(236, 240, 245), rgb(51, 50, 48), rgb(239, 237, 230);
    /// The command palette's selected row.
    PALETTE_HOT palette_hot: rgb(19, 42, 63), rgb(38, 43, 49), rgb(228, 236, 244), rgb(58, 57, 53), rgb(236, 230, 222);
    /// Dividers between rows of a dialog list.
    ROW_LINE row_line: rgb(19, 42, 61), rgb(37, 41, 47), rgb(228, 233, 239), rgb(56, 55, 51), rgb(232, 230, 220);
    /// The Activity list, its hovered row and the dividers between rows.
    WELL well: rgb(11, 24, 37), rgb(21, 23, 26), rgb(244, 246, 249), rgb(28, 27, 26), rgb(247, 246, 241);
    WELL_HOT well_hot: rgb(17, 37, 57), rgb(33, 37, 42), rgb(234, 239, 245), rgb(44, 43, 41), rgb(238, 236, 228);
    WELL_LINE well_line: rgb(18, 38, 58), rgb(30, 33, 37), rgb(228, 233, 239), rgb(46, 45, 43), rgb(232, 230, 220);
    /// A hovered repository tab.
    TAB_HOT tab_hot: rgb(12, 25, 37), rgb(24, 26, 29), rgb(230, 234, 239), rgb(34, 33, 31), rgb(234, 232, 223);
    /// Timeline dots and idle radio rings.
    RING ring: rgb(58, 90, 120), rgb(80, 88, 98), rgb(150, 164, 180), rgb(92, 91, 85), rgb(169, 166, 155);
    RING_IDLE ring_idle: rgb(74, 101, 128), rgb(95, 103, 113), rgb(140, 154, 170), rgb(107, 106, 99), rgb(157, 154, 144);
    /// Disabled icons and idle rings on cards.
    DIM dim: rgb(78, 104, 128), rgb(98, 106, 116), rgb(140, 154, 170), rgb(107, 106, 99), rgb(169, 166, 155);
    /// The circle behind an author's initials.
    AVATAR avatar: rgb(40, 67, 93), rgb(58, 64, 72), rgb(210, 220, 230), rgb(74, 73, 68), rgb(227, 224, 213);
    /// A switch's knob while on.
    KNOB knob: rgb(244, 248, 252), rgb(244, 246, 248), WHITE, rgb(250, 249, 245), rgb(255, 255, 255);
    /// The menu bar: a hovered row, a hovered title and their text.
    MENU_HOT menu_hot: rgb(30, 59, 87), rgb(45, 58, 70), rgb(220, 232, 242), rgb(74, 55, 48), rgb(241, 226, 218);
    MENU_TITLE_HOT menu_title_hot: rgb(27, 53, 80), rgb(42, 48, 56), rgb(226, 232, 239), rgb(58, 57, 53), rgb(234, 232, 223);
    MENU_HOT_TEXT menu_hot_text: WHITE, WHITE, rgb(22, 32, 42), rgb(250, 249, 245), rgb(20, 20, 19);

    // Diffs and syntax.
    ADDED_ROW added_row: rgb(15, 49, 38), rgb(22, 48, 38), rgb(233, 247, 240), rgb(31, 58, 37), rgb(230, 242, 227);
    REMOVED_ROW removed_row: rgb(55, 28, 40), rgb(56, 30, 36), rgb(253, 237, 240), rgb(67, 34, 39), rgb(250, 232, 230);
    ADDED_WORD added_word: rgb(29, 92, 67), rgb(28, 84, 62), rgb(189, 231, 209), rgb(45, 106, 59), rgb(191, 224, 185);
    REMOVED_WORD removed_word: rgb(113, 49, 67), rgb(100, 46, 56), rgb(247, 197, 207), rgb(122, 53, 66), rgb(242, 196, 190);
    /// Search matches in a diff: the current one and the others.
    MATCH match_fill: rgb(148, 102, 28), rgb(148, 102, 28), rgb(250, 214, 130), rgb(154, 111, 30), rgb(245, 208, 138);
    MATCH_DIM match_dim: rgb(73, 65, 35), rgb(73, 65, 35), rgb(250, 236, 190), rgb(74, 63, 34), rgb(247, 232, 196);
    SYNTAX_KEYWORD syntax_keyword: rgb(191, 162, 242), rgb(191, 162, 242), rgb(107, 74, 214), rgb(212, 155, 214), rgb(138, 63, 160);
    SYNTAX_STRING syntax_string: rgb(221, 190, 137), rgb(221, 190, 137), rgb(150, 96, 20), rgb(204, 181, 122), rgb(138, 100, 20);
    SYNTAX_COMMENT syntax_comment: rgb(126, 169, 143), rgb(126, 169, 143), rgb(88, 125, 100), rgb(140, 154, 126), rgb(107, 122, 94);
    SYNTAX_NUMBER syntax_number: rgb(128, 195, 221), rgb(128, 195, 221), rgb(0, 110, 150), rgb(227, 154, 122), rgb(180, 83, 47);
    SYNTAX_TYPE syntax_type: rgb(112, 208, 187), rgb(112, 208, 187), rgb(12, 120, 110), rgb(127, 196, 182), rgb(29, 127, 114);
    SYNTAX_FUNCTION syntax_function: rgb(116, 200, 237), rgb(116, 200, 237), rgb(10, 110, 160), rgb(141, 180, 226), rgb(47, 104, 168);
    SYNTAX_PROPERTY syntax_property: rgb(163, 195, 237), rgb(163, 195, 237), rgb(60, 90, 150), rgb(191, 198, 232), rgb(77, 90, 146);
    SYNTAX_ATTRIBUTE syntax_attribute: rgb(228, 172, 117), rgb(228, 172, 117), rgb(160, 90, 20), rgb(227, 176, 122), rgb(160, 90, 28);
    SYNTAX_OPERATOR syntax_operator: rgb(188, 200, 216), rgb(188, 200, 216), rgb(70, 85, 105), rgb(195, 194, 183), rgb(85, 84, 79);

    // Images.
    IMAGE_BG image_bg: rgb(19, 31, 44), rgb(28, 30, 34), rgb(236, 239, 243), rgb(35, 35, 33), rgb(240, 238, 230);
    /// The two squares of the transparency checkerboard.
    CHECKER_A checker_a: rgb(37, 51, 65), rgb(48, 51, 56), rgb(220, 224, 229), rgb(51, 50, 47), rgb(227, 225, 215);
    CHECKER_B checker_b: rgb(51, 67, 82), rgb(62, 66, 72), rgb(236, 239, 243), rgb(64, 63, 59), rgb(240, 238, 230);
    /// Pixels that differ between two image versions, and those that match.
    PIXEL_CHANGED pixel_changed: rgb(248, 78, 177), rgb(248, 78, 177), rgb(214, 40, 140), rgb(248, 78, 177), rgb(214, 40, 140);
    PIXEL_SAME pixel_same: rgb(32, 48, 65), rgb(40, 44, 50), rgb(226, 231, 237), rgb(46, 45, 43), rgb(232, 230, 220);

    // Terminal colours 0-15.
    ANSI_0 ansi_0: rgb(11, 24, 35), rgb(11, 24, 35), rgb(36, 41, 46), rgb(26, 25, 24), rgb(42, 41, 39);
    ANSI_1 ansi_1: rgb(219, 92, 100), rgb(219, 92, 100), rgb(207, 34, 46), rgb(229, 117, 108), rgb(200, 50, 63);
    ANSI_2 ansi_2: rgb(95, 195, 137), rgb(95, 195, 137), rgb(17, 99, 41), rgb(125, 187, 106), rgb(46, 125, 50);
    ANSI_3 ansi_3: rgb(218, 182, 95), rgb(218, 182, 95), rgb(122, 90, 0), rgb(217, 180, 95), rgb(138, 100, 20);
    ANSI_4 ansi_4: rgb(92, 164, 239), rgb(92, 164, 239), rgb(9, 105, 218), rgb(106, 155, 204), rgb(47, 104, 168);
    ANSI_5 ansi_5: rgb(184, 136, 213), rgb(184, 136, 213), rgb(130, 80, 223), rgb(192, 142, 214), rgb(138, 63, 160);
    ANSI_6 ansi_6: rgb(95, 197, 207), rgb(95, 197, 207), rgb(27, 124, 131), rgb(108, 191, 179), rgb(29, 127, 114);
    ANSI_7 ansi_7: rgb(207, 219, 229), rgb(207, 219, 229), rgb(110, 119, 129), rgb(214, 212, 203), rgb(115, 114, 108);
    ANSI_8 ansi_8: rgb(105, 126, 146), rgb(105, 126, 146), rgb(87, 96, 106), rgb(115, 114, 108), rgb(92, 91, 85);
    ANSI_9 ansi_9: rgb(249, 128, 132), rgb(249, 128, 132), rgb(164, 14, 38), rgb(240, 143, 134), rgb(168, 37, 47);
    ANSI_10 ansi_10: rgb(130, 224, 165), rgb(130, 224, 165), rgb(26, 127, 55), rgb(154, 211, 138), rgb(35, 122, 58);
    ANSI_11 ansi_11: rgb(249, 216, 134), rgb(249, 216, 134), rgb(99, 60, 1), rgb(235, 203, 128), rgb(110, 78, 0);
    ANSI_12 ansi_12: rgb(133, 191, 255), rgb(133, 191, 255), rgb(3, 102, 214), rgb(143, 184, 224), rgb(35, 84, 154);
    ANSI_13 ansi_13: rgb(219, 172, 244), rgb(219, 172, 244), rgb(130, 80, 223), rgb(215, 171, 234), rgb(122, 53, 144);
    ANSI_14 ansi_14: rgb(144, 230, 237), rgb(144, 230, 237), rgb(27, 124, 131), rgb(143, 214, 203), rgb(23, 107, 96);
    ANSI_15 ansi_15: rgb(245, 248, 252), rgb(245, 248, 252), rgb(140, 149, 159), rgb(250, 249, 245), rgb(143, 141, 133);
}

/// Graph lane colours, in lane order.
#[allow(non_snake_case)]
pub fn LANES() -> [Color32; 6] {
    [BLUE(), ORANGE(), PINK(), GREEN(), VIOLET(), TEAL()]
}

/// The statistics heat ramp, from no changes to the most.
#[allow(non_snake_case)]
pub fn RAMP() -> [Color32; 5] {
    [CHIP(), RAMP_1(), RAMP_2(), RAMP_3(), BLUE()]
}

/// The terminal's 16 colours.
#[allow(non_snake_case)]
pub fn ANSI() -> [Color32; 16] {
    [
        ANSI_0(), ANSI_1(), ANSI_2(), ANSI_3(), ANSI_4(), ANSI_5(), ANSI_6(), ANSI_7(),
        ANSI_8(), ANSI_9(), ANSI_10(), ANSI_11(), ANSI_12(), ANSI_13(), ANSI_14(), ANSI_15(),
    ]
}

static GENERATION: AtomicU64 = AtomicU64::new(0);

/// Makes `palette` the one every accessor reads, and returns the new generation.
pub(super) fn activate(palette: &Palette) -> u64 {
    palette.store();
    GENERATION.fetch_add(1, Relaxed) + 1
}

/// Changes whenever another palette is activated, so caches that hold coloured
/// layouts know to rebuild them.
pub fn generation() -> u64 {
    GENERATION.load(Relaxed)
}
