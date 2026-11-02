//! Preferences → Appearance: the five theme tiles, the Custom theme's colours and
//! picture, and row density.
use super::{Accent, Base, Choice, Look, Picture, Position, SWATCHES, Settings, Size, VISIBILITY, resolve};
use crate::{
    app::Rocket,
    pages,
    preferences::{ROW_DENSITIES, card_frame, card_header, card_note, choices, divider, status_line},
    style::*,
};
use eframe::egui::*;
use serde_json::{Value, json};

/// The label column of a card row.
const LABEL: f32 = 104.0;
const TILE: f32 = 110.0;
/// The picture part of a tile; its name sits below.
const THUMB: f32 = 78.0;
const GAP: f32 = 12.0;

impl Rocket {
    /// Returns whether Choose image was clicked.
    pub(crate) fn appearance_page(&mut self, ui: &mut Ui) -> bool {
        let ctx = ui.ctx().clone();
        let system = ctx.system_theme();
        let picture_accent = self.backdrop.accent(&self.theme.custom.picture);
        let widgets = &mut self.frame_widgets;
        let theme = &mut self.theme;
        let backdrop = &mut self.backdrop;
        card_frame(ui, |ui| {
            card_header(ui, "Theme", |_| {});
            Frame::new().inner_margin(Margin::same(16)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                let width = ((ui.available_width() - 4.0 * GAP) / 5.0).floor();
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = GAP;
                    for choice in Choice::ALL {
                        let looks = match choice {
                            Choice::System => vec![plain(Choice::Light), plain(Choice::Dark)],
                            Choice::Custom => vec![resolve(
                                &Settings { choice, custom: theme.custom.clone() },
                                system,
                                picture_accent,
                            )],
                            other => vec![plain(other)],
                        };
                        let picture = if choice == Choice::Custom && theme.custom.picture != Picture::None {
                            backdrop.tile(&ctx, &theme.custom.picture)
                        } else {
                            None
                        };
                        let r = tile(ui, width, choice.label(), theme.choice == choice, false, |painter, thumb| {
                            let parts = looks.len() as f32;
                            for (i, look) in looks.iter().enumerate() {
                                let x = |n: f32| thumb.left() + thumb.width() * n / parts;
                                let part = Rect::from_min_max(pos2(x(i as f32), thumb.top()), pos2(x(i as f32 + 1.0), thumb.bottom()));
                                let corners = CornerRadius {
                                    nw: if i == 0 { 7 } else { 0 },
                                    ne: if i + 1 == looks.len() { 7 } else { 0 },
                                    ..CornerRadius::ZERO
                                };
                                mini_window(painter, part, look, picture, corners);
                            }
                        });
                        pages::control(widgets, format!("theme-{}", choice.key()), &r, ui.clip_rect());
                        if r.clicked() {
                            theme.choice = choice;
                        }
                    }
                });
            });
        });
        let mut browse = false;
        if self.theme.choice == Choice::Custom {
            self.custom_colours(ui, picture_accent);
            browse = self.custom_background(ui);
        }
        self.row_density(ui);
        browse
    }

    /// The theme for automation: the one chosen and what the window shows; while Custom
    /// is chosen, its settings and picture. The user's own picture is named only by the
    /// file name Rocket keeps it under, never by a path.
    pub fn appearance_snapshot(&self) -> Value {
        let key = |base: Base| base.label().to_lowercase();
        let look = self.look.as_ref();
        // Rocket's palette on screen: System's follows Windows; Custom keeps its base's
        // background under the accent and see-through panes.
        let palette = look.and_then(|l| Base::ALL.into_iter().find(|b| b.palette().bg == l.palette.bg)).map(key);
        let windows = self.context.system_theme().map(|t| match t {
            Theme::Light => "light",
            Theme::Dark => "dark",
        });
        let custom = self.theme.custom().map(|c| {
            json!({
                "base": key(c.base),
                "accent": match c.accent {
                    Accent::FromPicture => "picture".to_owned(),
                    Accent::Colour(rgb) => hex(rgb),
                },
                "picture": c.picture.key(),
                "picture_name": match &c.picture {
                    Picture::Own(name) if super::picture::own_name(name) => Some(name.as_str()),
                    _ => None,
                },
                "size": match c.size {
                    Size::Fill => "fill",
                    Size::Fit => "fit",
                },
                "position": match c.position {
                    Position::Left => "left",
                    Position::Centre => "centre",
                    Position::Right => "right",
                },
                "visibility": c.visibility,
                "picture_shown": self.backdrop.shows(c),
                "picture_preparing": self.backdrop.busy(),
                "picture_problem": self.backdrop.problem(&c.picture),
            })
        });
        json!({
            "theme": self.theme.choice.key(),
            "palette": palette,
            "windows_mode": windows,
            "accent": look.map(|l| hex([l.palette.accent.r(), l.palette.accent.g(), l.palette.accent.b()])),
            "custom": custom,
            "importing": self.theme_import.is_some(),
            "import_error": self.theme_import_error,
        })
    }

    fn custom_colours(&mut self, ui: &mut Ui, picture_accent: Option<Color32>) {
        let widgets = &mut self.frame_widgets;
        let custom = &mut self.theme.custom;
        let code = &mut self.preferences.accent_code;
        card_frame(ui, |ui| {
            card_header(ui, "Colours", |_| {});
            row(ui, "Start from", |ui| {
                let items: Vec<(&str, bool)> = Base::ALL.iter().map(|b| (b.label(), custom.base == *b)).collect();
                let ids: Vec<String> = Base::ALL.iter().map(|b| format!("theme-base-{}", b.label().to_lowercase())).collect();
                if let Some(i) = choices(ui, widgets, &items, &ids) {
                    custom.base = Base::ALL[i];
                }
            });
            divider(ui);
            row(ui, "Accent", |ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                let r = from_picture(ui, custom.accent == Accent::FromPicture, picture_accent.unwrap_or(BLUE()));
                pages::control(widgets, "theme-accent-picture", &r, ui.clip_rect());
                if r.clicked() {
                    custom.accent = Accent::FromPicture;
                }
                separator(ui);
                for (name, rgb) in SWATCHES {
                    let r = swatch(ui, name, Color32::from_rgb(rgb[0], rgb[1], rgb[2]), custom.accent == Accent::Colour(rgb));
                    pages::control(widgets, format!("theme-accent-{}", name.to_lowercase()), &r, ui.clip_rect());
                    if r.clicked() {
                        custom.accent = Accent::Colour(rgb);
                    }
                }
                separator(ui);
                let edit = ui.add(
                    TextEdit::singleline(code)
                        .font(FontId::monospace(12.5))
                        .desired_width(76.0)
                        .char_limit(7)
                        .hint_text("#RRGGBB"),
                );
                pages::control(widgets, "theme-accent-code", &edit, ui.clip_rect());
                if edit.changed()
                    && let Some(rgb) = parse_code(code)
                {
                    custom.accent = Accent::Colour(rgb);
                }
                if !edit.has_focus() {
                    // The colour in use: the one chosen, or the picture's.
                    *code = match custom.accent {
                        Accent::Colour(rgb) => hex(rgb),
                        Accent::FromPicture => hex([BLUE().r(), BLUE().g(), BLUE().b()]),
                    };
                }
            });
        });
    }

    /// Returns whether Choose image was clicked.
    fn custom_background(&mut self, ui: &mut Ui) -> bool {
        let ctx = ui.ctx().clone();
        let mut browse = false;
        let widgets = &mut self.frame_widgets;
        let custom = &mut self.theme.custom;
        let backdrop = &mut self.backdrop;
        let importing = self.theme_import.is_some();
        let import_error = self.theme_import_error.as_deref();
        card_frame(ui, |ui| {
            card_header(ui, "Background", |_| {});
            Frame::new().inner_margin(Margin::same(16)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                let width = ((ui.available_width() - 5.0 * GAP) / 6.0).floor();
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = GAP;
                    let none = custom.picture == Picture::None;
                    let r = tile(ui, width, "None", none, false, |painter, thumb| {
                        painter.rect_filled(thumb, CornerRadius { nw: 7, ne: 7, ..CornerRadius::ZERO }, WINDOW());
                        icon(painter, Rect::from_center_size(thumb.center(), vec2(24.0, 24.0)), Icon::Blocked, FAINT());
                    });
                    pages::control(widgets, "theme-picture-none", &r, ui.clip_rect());
                    if r.clicked() {
                        custom.picture = Picture::None;
                    }
                    for picture in Picture::BUILT_IN {
                        let texture = backdrop.tile(&ctx, &picture);
                        let r = tile(ui, width, picture.label(), custom.picture == picture, false, |painter, thumb| {
                            cover(painter, thumb, texture);
                        });
                        pages::control(widgets, format!("theme-picture-{}", picture.key()), &r, ui.clip_rect());
                        if r.clicked() {
                            custom.picture = picture;
                        }
                    }
                    // The user's picture once chosen; choosing again replaces it.
                    let own = matches!(custom.picture, Picture::Own(_));
                    let texture = if own { backdrop.tile(&ctx, &custom.picture) } else { None };
                    let label = if own { "Your picture" } else { "Choose image" };
                    let r = tile(ui, width, label, own, !own, |painter, thumb| {
                        if texture.is_some() {
                            cover(painter, thumb, texture);
                        } else {
                            icon(painter, Rect::from_center_size(thumb.center(), vec2(24.0, 24.0)), Icon::Image, MUTED());
                        }
                    });
                    pages::control(widgets, if own { "theme-picture-own" } else { "theme-picture-choose" }, &r, ui.clip_rect());
                    if r.clicked() && !importing {
                        browse = true;
                    }
                });
            });
            divider(ui);
            ui.add_enabled_ui(custom.picture != Picture::None, |ui| {
                row(ui, "Size", |ui| {
                    let all = [(Size::Fill, "Fill"), (Size::Fit, "Fit")];
                    let items: Vec<(&str, bool)> = all.iter().map(|(s, label)| (*label, custom.size == *s)).collect();
                    let ids: Vec<String> = all.iter().map(|(_, label)| format!("theme-size-{}", label.to_lowercase())).collect();
                    if let Some(i) = choices(ui, widgets, &items, &ids) {
                        custom.size = all[i].0;
                    }
                });
                divider(ui);
                row(ui, "Position", |ui| {
                    let all = [(Position::Left, "Left"), (Position::Centre, "Centre"), (Position::Right, "Right")];
                    let items: Vec<(&str, bool)> = all.iter().map(|(p, label)| (*label, custom.position == *p)).collect();
                    let ids: Vec<String> = all.iter().map(|(_, label)| format!("theme-position-{}", label.to_lowercase())).collect();
                    if let Some(i) = choices(ui, widgets, &items, &ids) {
                        custom.position = all[i].0;
                    }
                });
                divider(ui);
                row(ui, "Visibility", |ui| {
                    ui.visuals_mut().selection.bg_fill = BLUE();
                    ui.spacing_mut().slider_width = (ui.available_width() - 64.0).clamp(120.0, 360.0);
                    let r = ui.add(Slider::new(&mut custom.visibility, VISIBILITY).show_value(false).trailing_fill(true));
                    pages::control(widgets, "theme-visibility", &r, ui.clip_rect());
                    ui.label(RichText::new(format!("{}%", custom.visibility)).size(12.0).color(MUTED()));
                });
            });
            let problem = import_error.or_else(|| backdrop.problem(&custom.picture));
            if importing || problem.is_some() {
                divider(ui);
                card_note(ui, |ui| match problem {
                    _ if importing => status_line(ui, MUTED(), "Copying the picture"),
                    Some(message) => status_line(ui, RED(), message),
                    None => {}
                });
            }
        });
        browse
    }

    fn row_density(&mut self, ui: &mut Ui) {
        let widgets = &mut self.frame_widgets;
        let row_height = &mut self.row_height;
        card_frame(ui, |ui| {
            card_header(ui, "Row density", |_| {});
            Frame::new().inner_margin(Margin::same(16)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                let items: Vec<(&str, bool)> = ROW_DENSITIES
                    .iter()
                    .map(|(_, value, label)| (*label, *row_height == *value))
                    .collect();
                let ids: Vec<String> = ROW_DENSITIES.iter().map(|(key, ..)| (*key).to_owned()).collect();
                if let Some(i) = choices(ui, widgets, &items, &ids) {
                    *row_height = ROW_DENSITIES[i].1;
                }
                ui.add_space(12.0);
                // A preview of the commit list at the chosen height.
                Frame::new()
                    .fill(BG())
                    .stroke(Stroke::new(1.0, HAIRLINE()))
                    .corner_radius(6)
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.spacing_mut().item_spacing.y = 0.0;
                        for (i, (text, sha)) in [
                            ("Add folder browser to the Open dialog", "4f1c2ab"),
                            ("Group diagnostics by session", "c71d5b0"),
                            ("Move MCP settings into Preferences", "2be94f3"),
                        ]
                        .into_iter()
                        .enumerate()
                        {
                            let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), *row_height), Sense::hover());
                            if i == 0 {
                                ui.painter().rect_filled(rect, 0.0, SELECTED());
                            }
                            ui.painter().circle_stroke(pos2(rect.left() + 16.0, rect.center().y), 4.0, Stroke::new(2.0, BLUE()));
                            ui.painter().text(pos2(rect.left() + 32.0, rect.center().y), Align2::LEFT_CENTER, text, FontId::proportional(13.0), TEXT());
                            ui.painter().text(pos2(rect.right() - 12.0, rect.center().y), Align2::RIGHT_CENTER, sha, FontId::monospace(12.0), HINT());
                        }
                    });
            });
        });
    }
}

/// What a plain (non-Custom) theme shows.
fn plain(choice: Choice) -> Look {
    resolve(&Settings { choice, ..Default::default() }, None, None)
}

/// A card row: its label in a fixed column, then the control.
fn row(ui: &mut Ui, label: &str, content: impl FnOnce(&mut Ui)) {
    Frame::new().inner_margin(Margin::symmetric(16, 10)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(vec2(LABEL, CONTROL), Sense::hover());
            ui.painter().text(rect.left_center(), Align2::LEFT_CENTER, label, FontId::proportional(13.0), MUTED());
            content(ui);
        });
    });
}

/// A choice tile: a picture (`draw` fills the top part) above its name; the chosen
/// one has the accent outline and a check, an action tile a dashed one.
fn tile(ui: &mut Ui, width: f32, name: &str, selected: bool, action: bool, draw: impl FnOnce(&Painter, Rect)) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(width, TILE), Sense::click());
    response.widget_info(|| WidgetInfo::selected(WidgetType::RadioButton, ui.is_enabled(), selected, name));
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let thumb = Rect::from_min_size(rect.min, vec2(width, THUMB));
        let label = Rect::from_min_max(pos2(rect.left(), thumb.bottom()), rect.max);
        if !action {
            painter.rect_filled(rect, 8.0, BG());
        }
        draw(painter, thumb.shrink(1.0));
        let strip = if selected {
            SELECTED()
        } else if action {
            Color32::TRANSPARENT
        } else {
            WINDOW()
        };
        painter.rect_filled(label.shrink(1.0), CornerRadius { sw: 7, se: 7, ..CornerRadius::ZERO }, strip);
        let hot = ui.is_enabled() && (response.hovered() || response.has_focus());
        let edge = if hot { EDGE_HOT() } else { CARD_STROKE() };
        if action {
            dashed(painter, label.x_range(), label.top(), edge);
        } else {
            painter.hline(label.x_range(), label.top() + 0.5, Stroke::new(1.0, CARD_LINE()));
        }
        let font = if selected { semibold(13.0) } else { FontId::proportional(13.0) };
        let ink = if action { BLUE() } else { TEXT() };
        painter
            .with_clip_rect(label.shrink(1.0).intersect(painter.clip_rect()))
            .text(pos2(label.left() + 10.0, label.center().y), Align2::LEFT_CENTER, name, font, ink);
        if selected {
            icon(painter, Rect::from_center_size(pos2(label.right() - 16.0, label.center().y), vec2(12.0, 12.0)), Icon::Check, BLUE());
            painter.rect_stroke(rect, 8.0, Stroke::new(2.0, BLUE()), StrokeKind::Inside);
        } else if action {
            dashed_rect(painter, rect, edge);
        } else {
            painter.rect_stroke(rect, 8.0, Stroke::new(1.0, edge), StrokeKind::Inside);
        }
    }
    response
}

/// A small window in a theme's colours: its sidebar and a few lines of text, over its
/// picture when the theme shows one.
fn mini_window(painter: &Painter, rect: Rect, look: &Look, picture: Option<(TextureId, Vec2)>, corners: CornerRadius) {
    let p = &look.palette;
    let see_through = picture.is_some();
    if see_through {
        cover(painter, rect, picture);
    } else {
        painter.rect_filled(rect, corners, p.bg);
    }
    let side = Rect::from_min_size(rect.min, vec2((rect.width() * 0.28).min(34.0), rect.height()));
    let content = Rect::from_min_max(pos2(side.right(), rect.top()), rect.max);
    if see_through {
        painter.rect_filled(content, CornerRadius { ne: corners.ne, ..CornerRadius::ZERO }, p.pane_bg);
    }
    painter.rect_filled(side, CornerRadius { nw: corners.nw, ..CornerRadius::ZERO }, p.pane_panel);
    painter.vline(side.right() - 0.5, side.y_range(), Stroke::new(1.0, p.hairline));
    painter.rect_filled(Rect::from_min_size(side.min + vec2(7.0, 12.0), vec2(side.width() - 14.0, 6.0)), 3.0, p.selected);
    let line = if see_through { p.muted.gamma_multiply(0.45) } else { p.hairline };
    let x = content.left() + 10.0;
    let width = (content.right() - x - 8.0).max(0.0);
    for (y, share, color) in [(14.0, 0.58, p.accent), (28.0, 0.54, line), (42.0, 0.38, line), (56.0, 0.48, line)] {
        painter.rect_filled(Rect::from_min_size(pos2(x, rect.top() + y), vec2(width * share, 6.0)), 3.0, color);
    }
}

/// Paints a picture over `rect`, cropped to cover it and keeping its right side,
/// where Rocket's pictures have their subject.
fn cover(painter: &Painter, rect: Rect, picture: Option<(TextureId, Vec2)>) {
    let corners = CornerRadius { nw: 7, ne: 7, ..CornerRadius::ZERO };
    let Some((texture, size)) = picture else {
        painter.rect_filled(rect, corners, WINDOW());
        return;
    };
    let (shape, image) = (rect.width() / rect.height(), size.x / size.y.max(1.0));
    let uv = if image > shape {
        Rect::from_min_max(pos2(1.0 - shape / image, 0.0), pos2(1.0, 1.0))
    } else {
        let top = (1.0 - image / shape) / 2.0;
        Rect::from_min_max(pos2(0.0, top), pos2(1.0, 1.0 - top))
    };
    painter.add(epaint::RectShape::filled(rect, corners, Color32::WHITE).with_texture(texture, uv));
}

/// The From picture choice: the picture's colour in a dot, and its name.
fn from_picture(ui: &mut Ui, selected: bool, colour: Color32) -> Response {
    let name = "From picture";
    let font = if selected { semibold(13.0) } else { FontId::proportional(13.0) };
    let galley = ui.painter().layout_no_wrap(name.into(), font, TEXT());
    let (rect, response) = ui.allocate_exact_size(vec2(galley.size().x + 40.0, CONTROL), Sense::click());
    response.widget_info(|| WidgetInfo::selected(WidgetType::RadioButton, ui.is_enabled(), selected, name));
    let hot = response.hovered() || response.has_focus();
    let painter = ui.painter();
    let edge = if selected { BLUE() } else if hot { EDGE_HOT() } else { BORDER() };
    painter.rect(rect, 14.0, if selected { SELECTED() } else { RAISED() }, Stroke::new(if selected { 2.0 } else { 1.0 }, edge), StrokeKind::Inside);
    painter.circle_filled(pos2(rect.left() + 15.0, rect.center().y), 9.0, colour);
    painter.galley(pos2(rect.left() + 30.0, rect.center().y - galley.size().y / 2.0), galley, TEXT());
    response
}

/// An accent swatch; the chosen one has a ring around it.
fn swatch(ui: &mut Ui, name: &str, colour: Color32, selected: bool) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::click());
    response.widget_info(|| WidgetInfo::selected(WidgetType::RadioButton, ui.is_enabled(), selected, name));
    let painter = ui.painter();
    painter.circle_filled(rect.center(), 11.0, colour);
    if selected {
        painter.circle_stroke(rect.center(), 14.0, Stroke::new(2.0, colour));
    } else if response.hovered() || response.has_focus() {
        painter.circle_stroke(rect.center(), 13.5, Stroke::new(1.0, EDGE_HOT()));
    }
    response.on_hover_text(name)
}

fn separator(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(1.0, 22.0), Sense::hover());
    ui.painter().vline(rect.center().x, rect.y_range(), Stroke::new(1.0, BORDER()));
}

fn dashed(painter: &Painter, x: Rangef, y: f32, colour: Color32) {
    painter.extend(Shape::dashed_line(&[pos2(x.min + 1.0, y), pos2(x.max - 1.0, y)], Stroke::new(1.0, colour), 4.0, 3.0));
}

fn dashed_rect(painter: &Painter, rect: Rect, colour: Color32) {
    let r = rect.shrink(0.5);
    let points = [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom(), r.left_top()];
    painter.extend(Shape::dashed_line(&points, Stroke::new(1.0, colour), 4.0, 3.0));
}

/// `#RRGGBB` or `RRGGBB`.
fn parse_code(text: &str) -> Option<[u8; 3]> {
    let text = text.trim();
    let digits = text.strip_prefix('#').unwrap_or(text);
    if digits.len() != 6 || !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let value = u32::from_str_radix(digits, 16).ok()?;
    Some([(value >> 16) as u8, (value >> 8) as u8, value as u8])
}

fn hex([r, g, b]: [u8; 3]) -> String {
    format!("#{r:02X}{g:02X}{b:02X}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colour_codes_parse_with_or_without_the_hash() {
        assert_eq!(parse_code("#F472B6"), Some([0xF4, 0x72, 0xB6]));
        assert_eq!(parse_code(" f472b6 "), Some([0xF4, 0x72, 0xB6]));
        assert_eq!(parse_code("#F472B"), None);
        assert_eq!(parse_code("#GG0000"), None);
        assert_eq!(hex([0xF4, 0x72, 0xB6]), "#F472B6");
    }

    /// Automation reads the palette that shows, and the user's picture only by the name
    /// Rocket keeps it under.
    #[test]
    fn the_snapshot_reports_what_shows_and_names_no_paths() {
        crate::test_common::isolated(
            "ROCKET_APPEARANCE_UI_CASE",
            "theme::appearance::tests::the_snapshot_reports_what_shows_and_names_no_paths",
            || {
                use super::super::Custom;
                let fixture = crate::test_common::Fixture::new();
                let ctx = Context::default();
                let mut app = Rocket::with_log_directory(&ctx, vec![], fixture.root.join("logs"));
                app.theme.choice = Choice::System;
                app.look = Some(resolve(&app.theme, Some(Theme::Light), None));
                let s = app.appearance_snapshot();
                assert_eq!((s["theme"].as_str(), s["palette"].as_str()), (Some("system"), Some("light")));
                assert!(s["custom"].is_null(), "Custom's settings show only while it is chosen");
                let name = "picture-0123456789abcdef.png";
                app.theme.choice = Choice::Custom;
                app.theme.custom = Custom {
                    base: Base::Dark,
                    accent: Accent::Colour([0xE8, 0x5D, 0x04]),
                    picture: Picture::Own(name.into()),
                    size: Size::Fit,
                    position: Position::Left,
                    visibility: 45,
                };
                app.look = Some(resolve(&app.theme, None, None));
                let s = app.appearance_snapshot();
                assert_eq!(s["palette"], "dark", "a Custom theme shows its base");
                assert_eq!(
                    s["custom"],
                    json!({"base": "dark", "accent": "#E85D04", "picture": "own", "picture_name": name,
                        "size": "fit", "position": "left", "visibility": 45,
                        "picture_shown": false, "picture_preparing": false, "picture_problem": null})
                );
                let folder = fixture.root.file_name().unwrap().to_string_lossy().into_owned();
                assert!(!s.to_string().contains(&folder), "no paths: {s}");
            },
        );
    }
}
