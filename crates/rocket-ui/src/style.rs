use eframe::egui::{self, *};
mod tokens;
pub use tokens::*;

/// Shared heights so rows and controls line up across panes.
pub const CONTROL: f32 = 28.0;

pub fn install(ctx: &Context) {
    // The default theme's dark mode until the first frame shows the chosen theme.
    apply(ctx, &Palette::CLAY_DARK, false);
    // Windows 10 repaints only part of an open window's title bar when its theme
    // changes, so dragging showed dark caption text on a light bar. The window is
    // created in the Windows theme; it keeps it.
    ctx.options_mut(|o| o.sync_window_theme = false);
    ctx.set_fonts(crate::fonts::definitions());
}

/// Shows `palette`: every colour accessor reads it from now on and egui's own widgets
/// are restyled to match. A `light` palette starts from egui's light visuals.
pub fn apply(ctx: &Context, palette: &Palette, light: bool) {
    activate(palette);
    let p = palette;
    let mut style = egui::Style {
        visuals: if light { Visuals::light() } else { Visuals::dark() },
        ..Default::default()
    };
    style.visuals.override_text_color = Some(p.text);
    style.visuals.panel_fill = p.bg;
    style.visuals.extreme_bg_color = p.bg;
    style.visuals.faint_bg_color = p.raised;
    style.visuals.selection.bg_fill = p.selected;
    style.visuals.selection.stroke = Stroke::new(1.0, p.accent);
    style.visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, p.border);
    style.visuals.widgets.noninteractive.fg_stroke.color = p.muted;
    style.visuals.widgets.inactive.bg_fill = p.raised;
    style.visuals.widgets.inactive.weak_bg_fill = p.raised;
    style.visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, p.border);
    style.visuals.widgets.inactive.fg_stroke.color = p.text;
    style.visuals.widgets.hovered.bg_fill = p.hover;
    style.visuals.widgets.hovered.weak_bg_fill = p.hover;
    style.visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, p.edge_hot);
    style.visuals.widgets.active.bg_fill = p.selected;
    style.visuals.widgets.active.weak_bg_fill = p.selected;
    style.visuals.widgets.active.fg_stroke.color = p.text;
    style.visuals.widgets.open.weak_bg_fill = p.raised;
    style.visuals.hyperlink_color = p.accent;
    style.visuals.indent_has_left_vline = false;
    style.visuals.window_corner_radius = CornerRadius::same(10);
    style.visuals.menu_corner_radius = CornerRadius::same(9);
    style.visuals.window_fill = p.window;
    style.visuals.window_stroke = Stroke::new(1.0, p.window_edge);
    style.spacing.item_spacing = vec2(8.0, 5.0);
    style.spacing.button_padding = vec2(10.0, 5.0);
    style.spacing.interact_size.y = CONTROL;
    style.visuals.widgets.inactive.corner_radius = CornerRadius::same(6);
    style.visuals.widgets.hovered.corner_radius = CornerRadius::same(6);
    style.visuals.widgets.active.corner_radius = CornerRadius::same(6);
    style.animation_time = 0.0;
    style
        .text_styles
        .insert(TextStyle::Body, FontId::proportional(13.0));
    style
        .text_styles
        .insert(TextStyle::Button, FontId::proportional(13.0));
    style
        .text_styles
        .insert(TextStyle::Small, FontId::proportional(12.0));
    style.text_styles.insert(TextStyle::Heading, semibold(18.0));
    style
        .text_styles
        .insert(TextStyle::Monospace, FontId::monospace(13.0));
    // Every theme uses egui's dark style slot, so switching never asks the window to
    // change its title bar (see `install`).
    ctx.set_theme(Theme::Dark);
    ctx.set_style_of(Theme::Dark, style);
    ctx.request_repaint();
}

pub fn semibold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("semibold".into()))
}

pub fn frame() -> Frame {
    Frame::new()
        .fill(PANEL())
        .stroke(Stroke::new(1.0, HAIRLINE()))
        .corner_radius(8)
        .inner_margin(12)
}

/// How a command is named in the menu bar and the command palette. Source labels may
/// end with the usual "opens a dialog" ellipsis; Rocket shows commands without it so
/// menus read as plain text (user preference). Only that one trailing `…` is removed.
pub fn command_label(label: &str) -> &str {
    label.strip_suffix('…').unwrap_or(label)
}

/// A checkbox with a square-cornered box. egui draws the box with the widget corner
/// radius, and the 6 px that buttons use turns its 14 px box into a radio-like
/// circle. The smaller radius applies only while the checkbox paints, in the same
/// `Ui`, so buttons and layout (including wrapping rows) are unchanged.
pub fn checkbox<'a>(checked: &'a mut bool, text: impl IntoAtoms<'a>) -> impl Widget + 'a {
    let checkbox = Checkbox::new(checked, text);
    move |ui: &mut Ui| {
        let saved = ui.visuals().widgets.clone();
        let widgets = &mut ui.visuals_mut().widgets;
        for visuals in [
            &mut widgets.noninteractive,
            &mut widgets.inactive,
            &mut widgets.hovered,
            &mut widgets.active,
        ] {
            visuals.corner_radius = CornerRadius::same(3);
        }
        let response = ui.add(checkbox);
        ui.visuals_mut().widgets = saved;
        response
    }
}

/// A button that shows its frame only when hovered, focused or pressed, at one size in
/// every state. egui 0.36 lays out an idle `frame_when_inactive(false)` button without
/// its 1 px inactive stroke but a hovered one with it, so those buttons grew 2 px on
/// hover and pushed their neighbours. Here the frame is always laid out; while idle its
/// fill and stroke are transparent.
pub fn quiet<'a>(button: Button<'a>) -> impl Widget + 'a {
    move |ui: &mut Ui| {
        let saved = ui.visuals().widgets.inactive;
        let idle = &mut ui.visuals_mut().widgets.inactive;
        idle.weak_bg_fill = Color32::TRANSPARENT;
        idle.bg_fill = Color32::TRANSPARENT;
        idle.bg_stroke.color = Color32::TRANSPARENT;
        let response = ui.add(button.frame_when_inactive(true));
        ui.visuals_mut().widgets.inactive = saved;
        response
    }
}

/// A framed button inside popups and menus, a [`quiet`] one everywhere else.
pub fn quiet_unless<'a>(framed: bool, button: Button<'a>) -> impl Widget + 'a {
    move |ui: &mut Ui| {
        if framed {
            ui.add(button)
        } else {
            ui.add(quiet(button))
        }
    }
}

/// An on/off switch for settings rows. The row shows the label; `label` names the
/// switch for screen readers and automation.
pub fn switch(ui: &mut Ui, on: &mut bool, label: &str) -> Response {
    let (rect, mut response) = ui.allocate_exact_size(vec2(34.0, 20.0), Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    let checked = *on;
    response.widget_info(|| {
        WidgetInfo::selected(WidgetType::Checkbox, ui.is_enabled(), checked, label)
    });
    if ui.is_rect_visible(rect) {
        let hot = response.hovered() || response.has_focus();
        let track = if checked {
            BLUE()
        } else if hot {
            EDGE_HOT()
        } else {
            BORDER()
        };
        ui.painter().rect_filled(rect, 10.0, track);
        let x = if checked {
            rect.right() - 10.0
        } else {
            rect.left() + 10.0
        };
        ui.painter().circle_filled(
            pos2(x, rect.center().y),
            7.0,
            if checked {
                KNOB()
            } else {
                MUTED()
            },
        );
        if response.has_focus() {
            ui.painter().rect_stroke(
                rect.expand(2.0),
                12.0,
                Stroke::new(1.5, BLUE()),
                StrokeKind::Outside,
            );
        }
    }
    response
}

/// How long ago something happened, for activity and folder lists: "just now",
/// "4 min ago", "3 h ago", "2 days ago", then the UTC date. Rocket has no local time
/// zone, so recent times stay relative rather than showing UTC clock times.
pub fn ago(age_seconds: u64, unix_seconds: i64) -> String {
    match age_seconds {
        0..45 => "just now".into(),
        45..3_600 => format!("{} min ago", (age_seconds / 60).max(1)),
        3_600..86_400 => format!("{} h ago", age_seconds / 3_600),
        86_400..172_800 => "1 day ago".into(),
        172_800..604_800 => format!("{} days ago", age_seconds / 86_400),
        _ => date(unix_seconds, 0)[..10].to_owned(),
    }
}

/// A rounded count pill such as the Changes badge or ahead/behind counts.
pub fn count_pill(painter: &Painter, center: Pos2, text: &str, color: Color32) -> Rect {
    let galley = painter.layout_no_wrap(text.into(), semibold(11.0), color);
    let size = vec2((galley.size().x + 10.0).max(18.0), 18.0);
    let rect = Rect::from_center_size(center, size);
    painter.rect_filled(rect, 9.0, color.gamma_multiply(0.16));
    painter.galley(
        pos2(
            rect.center().x - galley.size().x / 2.0,
            rect.center().y - galley.size().y / 2.0,
        ),
        galley,
        color,
    );
    rect
}

/// One-letter Git status badge: M modified, A added, D deleted, R renamed,
/// U untracked, C conflicted.
pub fn status_badge(painter: &Painter, center: Pos2, letter: char) {
    let color = status_color(letter);
    let rect = Rect::from_center_size(center, vec2(18.0, 18.0));
    if letter == 'C' {
        painter.rect_filled(rect, 4.0, RED());
    } else {
        painter.rect_filled(rect, 4.0, color.gamma_multiply(0.15));
    }
    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        letter,
        FontId::monospace(11.0),
        if letter == 'C' { TEXT() } else { color },
    );
}

pub fn status_color(letter: char) -> Color32 {
    match letter {
        'A' => GREEN(),
        'D' => RED(),
        'R' => VIOLET(),
        'U' => BLUE(),
        'C' => RED(),
        _ => AMBER(),
    }
}

/// Hover/selected background for list rows, with the single accent bar for selection.
pub fn row_background(painter: &Painter, rect: Rect, selected: bool, hovered: bool) {
    if selected {
        painter.rect_filled(rect, 6.0, SELECTED());
    } else if hovered {
        painter.rect_filled(rect, 6.0, HOVER());
    }
}

#[derive(Clone, Copy)]
pub enum Icon {
    Fetch,
    Pull,
    Push,
    Branch,
    Folder,
    Console,
    Prompt,
    File,
    Commit,
    Stash,
    Remote,
    Layout,
    Rocket,
    Tag,
    Modules,
    Search,
    Check,
    Chevron,
    Plug,
    Refresh,
    Activity,
    Plus,
    More,
    Database,
    Worktree,
    Lock,
    ChevronRight,
    ArrowLeft,
    ArrowUp,
    ArrowDown,
    Equal,
    NotEqual,
    Link,
    Shield,
    Pencil,
    Trash,
    Copy,
    External,
    Close,
    Sliders,
    Contrast,
    Drive,
    Home,
    Monitor,
    Eye,
    Info,
    Clock,
    Columns,
    Merge,
    Play,
    Stop,
    Warning,
    ChevronUp,
    Interrupt,
    Keyboard,
    User,
    Key,
    Image,
    Blocked,
    Puzzle,
    Sparkle,
    /// Three linked nodes: the repository map.
    Map,
    /// A box: a package in the map.
    Package,
    /// A compass: Explore, finding other people's repositories.
    Compass,
    /// A five-pointed star: a repository's stars.
    Star,
    /// A bookmark: Keep a preview.
    Bookmark,
    /// A ring around a play triangle: Actions, the host's CI runs.
    Actions,
}
pub fn icon(painter: &Painter, rect: Rect, icon: Icon, color: Color32) {
    let p = |x: f32, y: f32| {
        pos2(
            rect.left() + x * rect.width(),
            rect.top() + y * rect.height(),
        )
    };
    let line = |a, b| {
        painter.line_segment([a, b], Stroke::new(1.3, color));
    };
    match icon {
        Icon::Plug => {
            line(p(0.32, 0.05), p(0.32, 0.32));
            line(p(0.68, 0.05), p(0.68, 0.32));
            line(p(0.18, 0.32), p(0.82, 0.32));
            line(p(0.23, 0.32), p(0.23, 0.6));
            line(p(0.77, 0.32), p(0.77, 0.6));
            line(p(0.23, 0.6), p(0.5, 0.76));
            line(p(0.77, 0.6), p(0.5, 0.76));
            line(p(0.5, 0.76), p(0.5, 0.96));
        }
        Icon::Fetch => {
            // Cloud with a down arrow: download from the remote.
            painter.add(Shape::line(
                vec![
                    p(0.24, 0.66),
                    p(0.1, 0.58),
                    p(0.06, 0.44),
                    p(0.14, 0.31),
                    p(0.28, 0.28),
                    p(0.36, 0.14),
                    p(0.52, 0.08),
                    p(0.68, 0.14),
                    p(0.76, 0.28),
                    p(0.9, 0.32),
                    p(0.96, 0.46),
                    p(0.9, 0.6),
                    p(0.78, 0.66),
                ],
                Stroke::new(1.3, color),
            ));
            line(p(0.5, 0.42), p(0.5, 0.94));
            line(p(0.36, 0.8), p(0.5, 0.94));
            line(p(0.64, 0.8), p(0.5, 0.94));
        }
        Icon::Refresh => {
            let c = rect.center();
            let r = rect.width() * 0.36;
            let arc = |from: f32, to: f32| {
                let points: Vec<Pos2> = (0..=12)
                    .map(|i| {
                        let a = from + (to - from) * i as f32 / 12.0;
                        c + vec2(a.cos(), a.sin()) * r
                    })
                    .collect();
                painter.add(Shape::line(points, Stroke::new(1.3, color)));
            };
            arc(-2.6, 0.2);
            arc(0.54, 3.34);
            line(p(0.86, 0.36), p(0.86, 0.56));
            line(p(0.86, 0.56), p(0.68, 0.53));
            line(p(0.14, 0.64), p(0.14, 0.44));
            line(p(0.14, 0.44), p(0.32, 0.47));
        }
        Icon::Activity => {
            painter.add(Shape::line(
                vec![
                    p(0.04, 0.52),
                    p(0.26, 0.52),
                    p(0.38, 0.86),
                    p(0.6, 0.14),
                    p(0.72, 0.52),
                    p(0.96, 0.52),
                ],
                Stroke::new(1.3, color),
            ));
        }
        Icon::Plus => {
            line(p(0.5, 0.15), p(0.5, 0.85));
            line(p(0.15, 0.5), p(0.85, 0.5));
        }
        Icon::More => {
            for x in [0.2, 0.5, 0.8] {
                painter.circle_filled(p(x, 0.5), 1.4, color);
            }
        }
        Icon::Database => {
            let ellipse = |y: f32, full: bool| {
                let points: Vec<Pos2> = (0..=16)
                    .map(|i| {
                        let a = if full {
                            std::f32::consts::TAU * i as f32 / 16.0
                        } else {
                            std::f32::consts::PI * i as f32 / 16.0
                        };
                        p(0.5 + 0.38 * a.cos(), y + 0.12 * a.sin())
                    })
                    .collect();
                painter.add(Shape::line(points, Stroke::new(1.2, color)));
            };
            ellipse(0.2, true);
            ellipse(0.5, false);
            ellipse(0.8, false);
            line(p(0.12, 0.2), p(0.12, 0.8));
            line(p(0.88, 0.2), p(0.88, 0.8));
        }
        Icon::Worktree => {
            for (x, y) in [(0.22, 0.2), (0.78, 0.2), (0.5, 0.82)] {
                painter.circle_stroke(p(x, y), rect.width() * 0.12, Stroke::new(1.2, color));
            }
            line(p(0.22, 0.32), p(0.22, 0.44));
            line(p(0.78, 0.32), p(0.78, 0.44));
            line(p(0.22, 0.44), p(0.78, 0.44));
            line(p(0.5, 0.44), p(0.5, 0.7));
        }
        Icon::Lock => {
            painter.rect_stroke(
                Rect::from_min_max(p(0.18, 0.46), p(0.82, 0.92)),
                2.0,
                Stroke::new(1.3, color),
                StrokeKind::Inside,
            );
            painter.add(Shape::line(
                vec![
                    p(0.32, 0.46),
                    p(0.32, 0.28),
                    p(0.42, 0.12),
                    p(0.58, 0.12),
                    p(0.68, 0.28),
                    p(0.68, 0.46),
                ],
                Stroke::new(1.3, color),
            ));
        }
        Icon::ChevronRight => {
            line(p(0.4, 0.2), p(0.7, 0.5));
            line(p(0.7, 0.5), p(0.4, 0.8));
        }
        Icon::Pull | Icon::Push => {
            let flip = matches!(icon, Icon::Push);
            let y = if flip { 0.12 } else { 0.68 };
            let yy = if flip { 0.37 } else { 0.43 };
            line(p(0.5, 0.12), p(0.5, 0.68));
            line(p(0.25, yy), p(0.5, y));
            line(p(0.75, yy), p(0.5, y));
            line(p(0.15, 0.65), p(0.15, 0.9));
            line(p(0.15, 0.9), p(0.85, 0.9));
            line(p(0.85, 0.9), p(0.85, 0.65));
        }
        Icon::Branch => {
            line(p(0.25, 0.25), p(0.25, 0.8));
            line(p(0.25, 0.6), p(0.75, 0.4));
            line(p(0.75, 0.4), p(0.75, 0.25));
            for (x, y) in [(0.25, 0.18), (0.25, 0.85), (0.75, 0.18)] {
                painter.circle_stroke(p(x, y), 1.8, Stroke::new(1.0, color));
            }
        }
        Icon::Commit => {
            line(p(0.0, 0.5), p(0.32, 0.5));
            line(p(0.68, 0.5), p(1.0, 0.5));
            painter.circle_stroke(p(0.5, 0.5), rect.width() * 0.18, Stroke::new(1.3, color));
        }
        Icon::Folder => {
            let points = vec![
                p(0.07, 0.85),
                p(0.07, 0.18),
                p(0.43, 0.18),
                p(0.55, 0.32),
                p(0.93, 0.32),
                p(0.93, 0.85),
                p(0.07, 0.85),
            ];
            painter.add(Shape::line(points, Stroke::new(1.3, color)));
        }
        Icon::File => {
            painter.rect_stroke(
                Rect::from_min_max(p(0.2, 0.06), p(0.8, 0.94)),
                1.0,
                Stroke::new(1.3, color),
                StrokeKind::Inside,
            );
            line(p(0.35, 0.45), p(0.65, 0.45));
            line(p(0.35, 0.65), p(0.65, 0.65));
        }
        Icon::Console => {
            painter.rect_stroke(
                rect.shrink(1.0),
                1.0,
                Stroke::new(1.3, color),
                StrokeKind::Inside,
            );
            line(p(0.2, 0.3), p(0.4, 0.5));
            line(p(0.4, 0.5), p(0.2, 0.7));
            line(p(0.55, 0.7), p(0.8, 0.7));
        }
        Icon::Prompt => {
            line(p(0.14, 0.2), p(0.5, 0.5));
            line(p(0.5, 0.5), p(0.14, 0.8));
            line(p(0.54, 0.84), p(0.9, 0.84));
        }
        Icon::Stash => {
            painter.rect_stroke(
                Rect::from_min_max(p(0.15, 0.3), p(0.85, 0.9)),
                1.0,
                Stroke::new(1.3, color),
                StrokeKind::Inside,
            );
            line(p(0.05, 0.15), p(0.95, 0.15));
            line(p(0.35, 0.5), p(0.65, 0.5));
        }
        Icon::Remote => {
            painter.add(Shape::line(
                vec![
                    p(0.22, 0.8),
                    p(0.1, 0.7),
                    p(0.08, 0.5),
                    p(0.2, 0.38),
                    p(0.34, 0.38),
                    p(0.42, 0.2),
                    p(0.65, 0.2),
                    p(0.76, 0.4),
                    p(0.88, 0.43),
                    p(0.94, 0.57),
                    p(0.92, 0.72),
                    p(0.82, 0.8),
                    p(0.22, 0.8),
                ],
                Stroke::new(1.1, color),
            ));
        }
        Icon::Layout => {
            painter.rect_stroke(
                rect.shrink(1.0),
                1.0,
                Stroke::new(1.3, color),
                StrokeKind::Inside,
            );
            line(p(0.3, 0.1), p(0.3, 0.9));
        }
        Icon::Search => {
            painter.circle_stroke(p(0.43, 0.43), rect.width() * 0.31, Stroke::new(1.1, color));
            line(p(0.66, 0.66), p(0.93, 0.93));
        }
        Icon::Check => {
            line(p(0.12, 0.52), p(0.38, 0.78));
            line(p(0.38, 0.78), p(0.9, 0.2));
        }
        Icon::Chevron => {
            line(p(0.2, 0.4), p(0.5, 0.7));
            line(p(0.5, 0.7), p(0.8, 0.4));
        }
        Icon::ChevronUp => {
            line(p(0.2, 0.6), p(0.5, 0.3));
            line(p(0.5, 0.3), p(0.8, 0.6));
        }
        Icon::Interrupt => {
            painter.circle_stroke(rect.center(), rect.width() * 0.36, Stroke::new(1.3, color));
            line(p(0.36, 0.36), p(0.64, 0.64));
        }
        Icon::Keyboard => {
            // Its outline, a row of keys and the space bar.
            painter.rect_stroke(
                Rect::from_min_max(p(0.04, 0.22), p(0.96, 0.82)),
                2.0,
                Stroke::new(1.3, color),
                StrokeKind::Inside,
            );
            for x in [0.25, 0.42, 0.58, 0.75] {
                painter.circle_filled(p(x, 0.42), rect.width() * 0.05, color);
            }
            line(p(0.3, 0.63), p(0.7, 0.63));
        }
        Icon::User => {
            // A head and shoulders: hosting accounts.
            painter.circle_stroke(p(0.5, 0.3), rect.width() * 0.18, Stroke::new(1.3, color));
            let shoulders: Vec<Pos2> = (0..=16)
                .map(|i| {
                    let a = std::f32::consts::PI * (1.0 + i as f32 / 16.0);
                    p(0.5 + 0.36 * a.cos(), 0.95 + 0.36 * a.sin())
                })
                .collect();
            painter.add(Shape::line(shoulders, Stroke::new(1.3, color)));
        }
        Icon::Image => {
            // A framed picture: a sun over hills.
            painter.rect_stroke(
                Rect::from_min_max(p(0.08, 0.16), p(0.92, 0.84)),
                1.5,
                Stroke::new(1.3, color),
                StrokeKind::Inside,
            );
            painter.circle_stroke(p(0.36, 0.38), rect.width() * 0.08, Stroke::new(1.2, color));
            painter.add(Shape::line(
                vec![p(0.12, 0.74), p(0.38, 0.52), p(0.56, 0.68), p(0.7, 0.56), p(0.88, 0.72)],
                Stroke::new(1.3, color),
            ));
        }
        Icon::Puzzle => {
            // A jigsaw piece, with a tab on top and a socket on the right: extensions.
            let arc = |x: f32, y: f32, radius: f32, from: f32, to: f32| {
                (0..=10).map(move |i| {
                    let angle = (from + (to - from) * i as f32 / 10.0).to_radians();
                    (x + radius * angle.cos(), y + radius * angle.sin())
                })
            };
            let outline = [(0.16, 0.33)]
                .into_iter()
                .chain(arc(0.45, 0.205, 0.135, 112.0, 428.0))
                .chain([(0.74, 0.33)])
                .chain(arc(0.625, 0.56, 0.13, -28.0, -332.0))
                .chain([(0.74, 0.84), (0.16, 0.84), (0.16, 0.33)]);
            painter.add(Shape::line(
                outline.map(|(x, y)| p(x, y)).collect(),
                Stroke::new(1.3, color),
            ));
        }
        Icon::Blocked => {
            // A circle crossed through: none.
            painter.circle_stroke(rect.center(), rect.width() * 0.36, Stroke::new(1.3, color));
            line(p(0.25, 0.75), p(0.75, 0.25));
        }
        Icon::Key => {
            // A ring and a bit with two teeth: SSH keys.
            painter.circle_stroke(p(0.3, 0.7), rect.width() * 0.19, Stroke::new(1.3, color));
            line(p(0.44, 0.56), p(0.9, 0.1));
            line(p(0.77, 0.23), p(0.9, 0.36));
            line(p(0.64, 0.36), p(0.74, 0.46));
        }
        Icon::Tag => {
            painter.add(Shape::line(
                vec![
                    p(0.08, 0.12),
                    p(0.5, 0.12),
                    p(0.95, 0.57),
                    p(0.55, 0.96),
                    p(0.08, 0.5),
                    p(0.08, 0.12),
                ],
                Stroke::new(1.1, color),
            ));
            painter.circle_stroke(p(0.3, 0.33), 1.2, Stroke::new(1.0, color));
        }
        Icon::Modules => {
            for (x, y) in [(0.05, 0.05), (0.58, 0.05), (0.05, 0.58), (0.58, 0.58)] {
                painter.rect_stroke(
                    Rect::from_min_max(p(x, y), p(x + 0.36, y + 0.36)),
                    0.5,
                    Stroke::new(1.0, color),
                    StrokeKind::Inside,
                );
            }
        }
        Icon::Rocket => {
            crate::app_icon::paint_mark(painter, rect, Some(color));
        }
        Icon::ArrowLeft | Icon::ArrowUp | Icon::ArrowDown => {
            // Drawn pointing left; up swaps the axes and down also mirrors them.
            let q = |x: f32, y: f32| match icon {
                Icon::ArrowUp => p(y, x),
                Icon::ArrowDown => p(y, 1.0 - x),
                _ => p(x, y),
            };
            line(q(0.88, 0.5), q(0.12, 0.5));
            line(q(0.4, 0.22), q(0.12, 0.5));
            line(q(0.4, 0.78), q(0.12, 0.5));
        }
        Icon::Equal => {
            line(p(0.16, 0.36), p(0.84, 0.36));
            line(p(0.16, 0.64), p(0.84, 0.64));
        }
        Icon::NotEqual => {
            line(p(0.16, 0.36), p(0.84, 0.36));
            line(p(0.16, 0.64), p(0.84, 0.64));
            line(p(0.66, 0.14), p(0.34, 0.86));
        }
        Icon::Shield => {
            painter.add(Shape::line(
                vec![p(0.5, 0.08), p(0.86, 0.22), p(0.86, 0.5), p(0.5, 0.92), p(0.14, 0.5), p(0.14, 0.22), p(0.5, 0.08)],
                Stroke::new(1.3, color),
            ));
        }
        Icon::Link => {
            // Two links of a chain on a diagonal.
            let stroke = Stroke::new(1.3, color);
            painter.circle_stroke(p(0.34, 0.66), rect.width() * 0.22, stroke);
            painter.circle_stroke(p(0.66, 0.34), rect.width() * 0.22, stroke);
            line(p(0.4, 0.6), p(0.6, 0.4));
        }
        Icon::Pencil => {
            painter.add(Shape::line(
                vec![p(0.12, 0.88), p(0.12, 0.66), p(0.66, 0.12), p(0.88, 0.34), p(0.34, 0.88), p(0.12, 0.88)],
                Stroke::new(1.3, color),
            ));
            line(p(0.55, 0.23), p(0.77, 0.45));
        }
        Icon::Trash => {
            line(p(0.1, 0.24), p(0.9, 0.24));
            painter.add(Shape::line(
                vec![p(0.36, 0.24), p(0.36, 0.1), p(0.64, 0.1), p(0.64, 0.24)],
                Stroke::new(1.3, color),
            ));
            painter.add(Shape::line(
                vec![p(0.2, 0.24), p(0.27, 0.92), p(0.73, 0.92), p(0.8, 0.24)],
                Stroke::new(1.3, color),
            ));
        }
        Icon::Copy => {
            painter.rect_stroke(
                Rect::from_min_max(p(0.34, 0.34), p(0.92, 0.92)),
                1.5,
                Stroke::new(1.3, color),
                StrokeKind::Inside,
            );
            painter.add(Shape::line(
                vec![p(0.12, 0.7), p(0.12, 0.12), p(0.7, 0.12)],
                Stroke::new(1.3, color),
            ));
        }
        Icon::External => {
            painter.add(Shape::line(
                vec![p(0.58, 0.1), p(0.9, 0.1), p(0.9, 0.42)],
                Stroke::new(1.3, color),
            ));
            line(p(0.9, 0.1), p(0.45, 0.55));
            painter.add(Shape::line(
                vec![p(0.74, 0.6), p(0.74, 0.9), p(0.1, 0.9), p(0.1, 0.26), p(0.4, 0.26)],
                Stroke::new(1.3, color),
            ));
        }
        Icon::Close => {
            line(p(0.18, 0.18), p(0.82, 0.82));
            line(p(0.82, 0.18), p(0.18, 0.82));
        }
        Icon::Sliders => {
            for (y, knob) in [(0.22, 0.66), (0.5, 0.34), (0.78, 0.7)] {
                line(p(0.06, y), p(0.94, y));
                painter.circle_filled(p(knob, y), rect.width() * 0.11, color);
            }
        }
        Icon::Contrast => {
            let c = rect.center();
            let r = rect.width() * 0.42;
            painter.circle_stroke(c, r, Stroke::new(1.3, color));
            let half: Vec<Pos2> = (0..=12)
                .map(|i| {
                    let a = -std::f32::consts::FRAC_PI_2 + std::f32::consts::PI * i as f32 / 12.0;
                    c + vec2(a.cos(), a.sin()) * r
                })
                .collect();
            painter.add(Shape::convex_polygon(half, color, Stroke::NONE));
        }
        Icon::Drive => {
            painter.rect_stroke(
                Rect::from_min_max(p(0.06, 0.3), p(0.94, 0.72)),
                2.0,
                Stroke::new(1.3, color),
                StrokeKind::Inside,
            );
            painter.circle_filled(p(0.74, 0.51), 1.3, color);
        }
        Icon::Home => {
            painter.add(Shape::line(
                vec![p(0.08, 0.48), p(0.5, 0.1), p(0.92, 0.48)],
                Stroke::new(1.3, color),
            ));
            painter.add(Shape::line(
                vec![p(0.2, 0.38), p(0.2, 0.9), p(0.8, 0.9), p(0.8, 0.38)],
                Stroke::new(1.3, color),
            ));
        }
        Icon::Monitor => {
            painter.rect_stroke(
                Rect::from_min_max(p(0.06, 0.12), p(0.94, 0.68)),
                1.5,
                Stroke::new(1.3, color),
                StrokeKind::Inside,
            );
            line(p(0.5, 0.68), p(0.5, 0.86));
            line(p(0.28, 0.9), p(0.72, 0.9));
        }
        Icon::Eye => {
            painter.add(Shape::line(
                vec![
                    p(0.04, 0.5),
                    p(0.24, 0.27),
                    p(0.5, 0.19),
                    p(0.76, 0.27),
                    p(0.96, 0.5),
                    p(0.76, 0.73),
                    p(0.5, 0.81),
                    p(0.24, 0.73),
                    p(0.04, 0.5),
                ],
                Stroke::new(1.2, color),
            ));
            painter.circle_stroke(rect.center(), rect.width() * 0.14, Stroke::new(1.2, color));
        }
        Icon::Bookmark => {
            painter.add(Shape::closed_line(
                vec![p(0.26, 0.12), p(0.74, 0.12), p(0.74, 0.9), p(0.5, 0.68), p(0.26, 0.9)],
                Stroke::new(1.3, color),
            ));
        }
        Icon::Compass => {
            painter.circle_stroke(rect.center(), rect.width() * 0.43, Stroke::new(1.2, color));
            painter.add(Shape::closed_line(
                vec![p(0.68, 0.32), p(0.56, 0.56), p(0.32, 0.68), p(0.44, 0.44)],
                Stroke::new(1.2, color),
            ));
        }
        Icon::Star => {
            let center = rect.center();
            let (outer, inner) = (rect.width() * 0.46, rect.width() * 0.2);
            let points = (0..10)
                .map(|i| {
                    let angle = std::f32::consts::PI * (i as f32 / 5.0) - std::f32::consts::FRAC_PI_2;
                    let radius = if i % 2 == 0 { outer } else { inner };
                    center + vec2(angle.cos(), angle.sin()) * radius
                })
                .collect();
            painter.add(Shape::closed_line(points, Stroke::new(1.1, color)));
        }
        Icon::Info | Icon::Clock => {
            painter.circle_stroke(rect.center(), rect.width() * 0.43, Stroke::new(1.2, color));
            if matches!(icon, Icon::Info) {
                line(p(0.5, 0.44), p(0.5, 0.74));
                painter.circle_filled(p(0.5, 0.29), 1.1, color);
            } else {
                painter.add(Shape::line(
                    vec![p(0.5, 0.24), p(0.5, 0.5), p(0.69, 0.63)],
                    Stroke::new(1.2, color),
                ));
            }
        }
        Icon::Columns => {
            for (left, right) in [(0.06, 0.44), (0.56, 0.94)] {
                painter.rect_stroke(
                    Rect::from_min_max(p(left, 0.1), p(right, 0.9)),
                    1.0,
                    Stroke::new(1.2, color),
                    StrokeKind::Inside,
                );
            }
        }
        Icon::Sparkle => {
            // A large star and a small one, as closed outlines.
            for (cx, cy, r, k) in [(0.5, 0.44, 0.31, 0.09), (0.78, 0.84, 0.125, 0.0375)] {
                let tip = |dx: f32, dy: f32| p(cx + dx, cy + dy);
                let points = vec![
                    tip(0.0, -r), tip(k, -k), tip(r, 0.0), tip(k, k),
                    tip(0.0, r), tip(-k, k), tip(-r, 0.0), tip(-k, -k),
                ];
                painter.add(Shape::closed_line(points, Stroke::new(1.2, color)));
            }
        }
        Icon::Merge => {
            for (x, y) in [(0.25, 0.16), (0.25, 0.86), (0.78, 0.62)] {
                painter.circle_stroke(p(x, y), 1.8, Stroke::new(1.0, color));
            }
            line(p(0.25, 0.26), p(0.25, 0.76));
            painter.add(Shape::line(
                vec![p(0.25, 0.3), p(0.31, 0.48), p(0.46, 0.58), p(0.68, 0.62)],
                Stroke::new(1.2, color),
            ));
        }
        Icon::Actions => {
            let c = rect.center();
            let r = rect.width().min(rect.height()) * 0.44;
            painter.circle_stroke(c, r, Stroke::new(1.3, color));
            let s = r * 0.5;
            painter.add(Shape::convex_polygon(vec![c + vec2(-s * 0.7, -s), c + vec2(s, 0.0), c + vec2(-s * 0.7, s)], color, Stroke::NONE));
        }
        Icon::Play => {
            painter.add(Shape::convex_polygon(
                vec![p(0.24, 0.14), p(0.86, 0.5), p(0.24, 0.86)],
                color,
                Stroke::NONE,
            ));
        }
        Icon::Stop => {
            painter.rect_filled(Rect::from_min_max(p(0.2, 0.2), p(0.8, 0.8)), 1.5, color);
        }
        Icon::Map => {
            let nodes = [p(0.18, 0.24), p(0.82, 0.24), p(0.5, 0.8)];
            for (a, b) in [(0, 1), (0, 2), (1, 2)] {
                let (from, to) = (nodes[a], nodes[b]);
                let step = (to - from).normalized() * rect.width() * 0.14;
                line(from + step, to - step);
            }
            for n in nodes {
                painter.circle_stroke(n, rect.width() * 0.12, Stroke::new(1.3, color));
            }
        }
        Icon::Package => {
            painter.add(Shape::closed_line(
                vec![p(0.5, 0.06), p(0.9, 0.28), p(0.9, 0.72), p(0.5, 0.94), p(0.1, 0.72), p(0.1, 0.28)],
                Stroke::new(1.2, color),
            ));
            line(p(0.1, 0.28), p(0.5, 0.5));
            line(p(0.9, 0.28), p(0.5, 0.5));
            line(p(0.5, 0.5), p(0.5, 0.94));
        }
        Icon::Warning => {
            painter.add(Shape::line(
                vec![p(0.5, 0.08), p(0.95, 0.9), p(0.05, 0.9), p(0.5, 0.08)],
                Stroke::new(1.2, color),
            ));
            line(p(0.5, 0.38), p(0.5, 0.62));
            painter.circle_filled(p(0.5, 0.76), 1.1, color);
        }
    }
}
pub fn search_field(ui: &mut Ui, value: &mut String, hint: &str, width: f32) -> Response {
    Frame::new()
        .fill(RAISED())
        .stroke(Stroke::new(1.0, BORDER()))
        .corner_radius(6)
        .inner_margin(Margin::symmetric(10, 7))
        .show(ui, |ui| {
            ui.set_width((width - 22.0).max(30.0));
            ui.spacing_mut().interact_size.y = 22.0;
            ui.horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
                icon(ui.painter(), rect, Icon::Search, MUTED());
                ui.add(
                    TextEdit::singleline(value)
                        .frame(Frame::NONE)
                        .min_size(vec2(0.0, 22.0))
                        .hint_text(hint)
                        .desired_width(ui.available_width()),
                )
            })
            .inner
        })
        .inner
}

pub fn clipped_text(ui: &Ui, rect: Rect, text: &str, size: f32, color: Color32) {
    let mut job =
        text::LayoutJob::simple_singleline(text.into(), FontId::proportional(size), color);
    job.wrap = text::TextWrapping {
        max_width: rect.width().max(0.0),
        max_rows: 1,
        break_anywhere: true,
        ..Default::default()
    };
    let galley = ui.painter().layout_job(job);
    ui.painter()
        .with_clip_rect(rect.intersect(ui.clip_rect()))
        .galley(
            pos2(rect.left(), rect.center().y - galley.size().y / 2.0),
            galley,
            color,
        );
}

pub fn display(bytes: &[u8]) -> String {
    const LIMIT: usize = 16 * 1024;
    let mut text = String::from_utf8_lossy(crate::text::prefix(bytes, LIMIT)).into_owned();
    if bytes.len() > LIMIT {
        text.push_str("\n[Display limited to 16 KiB]");
    }
    text
}
pub fn subject(commit: &rocket_git::Commit) -> String {
    let bytes = crate::text::prefix(&commit.message, 4096);
    let mut text = String::from_utf8_lossy(bytes.split(|b| *b == b'\n').next().unwrap_or_default())
        .into_owned();
    if commit.message.len() > bytes.len() && !bytes.contains(&b'\n') {
        text.push_str("… [subject display limited]");
    }
    text
}
pub fn short_ref(bytes: &[u8]) -> String {
    display(bytes)
        .trim_start_matches("refs/heads/")
        .trim_start_matches("refs/remotes/")
        .to_owned()
}
pub fn head_name(head: Option<&rocket_git::Head>) -> String {
    match head {
        Some(rocket_git::Head::Branch { name, .. }) => display(name),
        Some(rocket_git::Head::Unborn { branch }) => format!("{} · unborn", display(branch)),
        Some(rocket_git::Head::Detached { target }) => {
            format!("{} · detached", &target.as_str()[..8])
        }
        None => "Unavailable".into(),
    }
}

pub fn date(timestamp: i64, offset: i32) -> String {
    // Gregorian civil date; use the author's recorded timezone, never invent local dates.
    let seconds = timestamp.saturating_add(i64::from(offset) * 60);
    let days = seconds.div_euclid(86400);
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let mut year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    let minutes = seconds.rem_euclid(86400) / 60;
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}",
        minutes / 60,
        minutes % 60
    )
}

#[cfg(test)]
mod unicode_tests {
    use super::*;

    #[test]
    #[cfg(windows)]
    fn installed_fonts_cover_unicode_in_every_ui_family() {
        let ctx = Context::default();
        install(&ctx);
        let mut output = ctx.run_ui(Default::default(), |ui| {
            ui.ctx().fonts_mut(|fonts| {
                for font in [
                    FontId::proportional(13.0),
                    FontId::monospace(13.0),
                    semibold(18.0),
                ] {
                    for ch in
                        "简体中文繁體漢字日本語𠮷ひらがなカタカナ한국어한글ÉlodieАннаe\u{301}🚀"
                            .chars()
                    {
                        // Check the real character maps: egui 0.36's has_glyph
                        // also returns false for supported characters in the
                        // same face that supplies the replacement glyph.
                        assert!(
                            fonts
                                .fonts
                                .font(&font.family)
                                .characters()
                                .contains_key(&ch),
                            "missing {ch:?} in {:?}",
                            font.family
                        );
                    }
                }
            });
        });
        output.textures_delta.clear();
    }

    #[test]
    fn unicode_display_does_not_split_a_character_at_the_limit() {
        let message = format!("{}文after", "a".repeat(16 * 1024 - 1));
        let shown = display(message.as_bytes());
        assert!(!shown.contains('\u{fffd}'));
        assert!(shown.ends_with("\n[Display limited to 16 KiB]"));
    }
}

#[cfg(test)]
mod checkbox_tests {
    use super::*;

    fn rounded(shape: &Shape, radii: &mut Vec<CornerRadius>) {
        match shape {
            Shape::Vec(shapes) => shapes.iter().for_each(|s| rounded(s, radii)),
            Shape::Rect(rect) if rect.corner_radius != CornerRadius::ZERO => {
                radii.push(rect.corner_radius);
            }
            _ => {}
        }
    }

    #[test]
    fn checkbox_box_is_square_cornered_and_buttons_keep_their_radius() {
        let ctx = Context::default();
        install(&ctx);
        let mut checked = false;
        let mut output = ctx.run_ui(Default::default(), |ui| {
            // A wrapping row, like the diff toolbar.
            ui.horizontal_wrapped(|ui| {
                ui.add(checkbox(&mut checked, "Folder"));
                let _ = ui.button("Load HEAD");
            });
            assert_eq!(
                ui.visuals().widgets.inactive.corner_radius,
                CornerRadius::same(6),
                "the checkbox restores the shared widget radius"
            );
        });
        output.textures_delta.clear();
        let mut radii = Vec::new();
        for clipped in &output.shapes {
            rounded(&clipped.shape, &mut radii);
        }
        assert_eq!(radii, [CornerRadius::same(3), CornerRadius::same(6)]);
    }
}

#[cfg(test)]
mod widget_tests {
    use super::*;

    /// Lays out two buttons side by side, with the pointer over the first after the
    /// first frame, and returns the second button's position in each frame.
    fn neighbour_positions(make: fn(&'static str) -> Button<'static>, quiet_buttons: bool) -> Vec<Pos2> {
        let ctx = Context::default();
        install(&ctx);
        let mut first = Rect::NOTHING;
        let mut positions = Vec::new();
        for frame in 0..4 {
            let mut input = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(480.0, 120.0))),
                ..Default::default()
            };
            if frame > 0 {
                input.events.push(Event::PointerMoved(first.center()));
            }
            let mut output = ctx.run_ui(input, |ui| {
                ui.horizontal(|ui| {
                    let (a, b) = if quiet_buttons {
                        (ui.add(quiet(make("Clear"))), ui.add(quiet(make("All diagnostics"))))
                    } else {
                        (ui.add(make("Clear")), ui.add(make("All diagnostics")))
                    };
                    first = a.rect;
                    positions.push(b.rect.min);
                });
            });
            output.textures_delta.clear();
        }
        positions
    }

    #[test]
    fn quiet_buttons_keep_their_size_when_hovered() {
        let framed_on_hover = |label| Button::new(label).frame_when_inactive(false);
        let raw = neighbour_positions(framed_on_hover, false);
        // egui 0.36 lays out the idle button without its 1 px stroke. If an egui update
        // stops this, `quiet` is no longer needed.
        assert_ne!(raw.first(), raw.last(), "egui moved the neighbour on hover");
        let stable = neighbour_positions(Button::new, true);
        assert!(
            stable.iter().all(|p| *p == stable[0]),
            "a hovered quiet button moved its neighbour: {stable:?}"
        );
    }

    #[test]
    fn ages_read_as_minutes_hours_days_then_dates() {
        let unix = 1_792_390_000; // 2026-10-19 UTC
        assert_eq!(ago(10, unix), "just now");
        assert_eq!(ago(50, unix), "1 min ago");
        assert_eq!(ago(59 * 60 + 59, unix), "59 min ago");
        assert_eq!(ago(3 * 3600 + 5, unix), "3 h ago");
        assert_eq!(ago(30 * 3600, unix), "1 day ago");
        assert_eq!(ago(3 * 86_400, unix), "3 days ago");
        assert_eq!(ago(8 * 86_400, unix), "2026-10-19");
    }

    #[test]
    fn the_window_title_bar_keeps_the_system_theme() {
        let ctx = Context::default();
        install(&ctx);
        let mut output = ctx.run_ui(RawInput::default(), |_| {});
        output.textures_delta.clear();
        let commands = output
            .viewport_output
            .get(&ViewportId::ROOT)
            .map_or(&[][..], |viewport| &viewport.commands[..]);
        assert!(
            !commands.iter().any(|c| matches!(c, ViewportCommand::SetTheme(_))),
            "the open window is asked to change its title bar theme: {commands:?}"
        );
    }
}
