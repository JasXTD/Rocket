//! About › Legal: Rocket's license and the notices of the software it includes, from
//! `LICENSE-MIT`, `LICENSE-APACHE` and `THIRD-PARTY-NOTICES.txt` at the workspace root.
//! The notices cover Windows, Linux and macOS; Legal lists the running platform's entries.
//! After changing dependencies, regenerate the notices with
//! `cargo run -p rocket-ui --example third_party_notices`.
use crate::{
    app::UiControl,
    pages::{self, control},
    style::*,
};
use eframe::egui::*;
use std::{collections::HashMap, sync::OnceLock};

const LICENSE_MIT: &str = include_str!("../../../LICENSE-MIT");
const LICENSE_APACHE: &str = include_str!("../../../LICENSE-APACHE");
const NOTICES: &str = include_str!("../../../THIRD-PARTY-NOTICES.txt");
/// Opens and closes each entry's header in the notices file.
const SEPARATOR: &str = "########################################################################";
/// Where the open dialog keeps its choices, in egui's memory.
const STATE: &str = "legal-state";
/// The name the notices give this build's platform. A platform Rocket ships no package
/// for has none and lists every entry.
const PLATFORM: Option<&str> = if cfg!(windows) {
    Some("Windows")
} else if cfg!(target_os = "linux") {
    Some("Linux")
} else if cfg!(target_os = "macos") {
    Some("macOS")
} else {
    None
};

/// One entry of THIRD-PARTY-NOTICES.txt.
pub(crate) struct Notice {
    /// `crate`, `font` or `runtime`.
    pub kind: String,
    pub name: String,
    pub version: String,
    pub license: String,
    /// Comma-separated build platforms: Windows, Linux and/or macOS.
    pub platforms: String,
    pub source: String,
    pub text: String,
}

impl Notice {
    /// Whether `platform`'s builds include it.
    fn ships_on(&self, platform: &str) -> bool {
        self.platforms.split(", ").any(|p| p == platform)
    }
}

/// The notices file's entries for this build's platform, read once.
pub(crate) fn notices() -> &'static [Notice] {
    static PARSED: OnceLock<Vec<Notice>> = OnceLock::new();
    PARSED.get_or_init(|| parse(NOTICES).into_iter().filter(|n| PLATFORM.is_none_or(|p| n.ships_on(p))).collect())
}

fn parse(file: &str) -> Vec<Notice> {
    let file = file.replace("\r\n", "\n");
    let separator = format!("{SEPARATOR}\n");
    let mut parts = file.split(separator.as_str()).skip(1);
    let mut notices = Vec::new();
    while let (Some(header), Some(body)) = (parts.next(), parts.next()) {
        let field = |key: &str| {
            header.lines().find_map(|l| l.strip_prefix(key)).map(str::trim).unwrap_or_default().to_owned()
        };
        notices.push(Notice {
            kind: field("Kind:"),
            name: field("Name:"),
            version: field("Version:"),
            license: field("License:"),
            platforms: field("Platforms:"),
            source: field("Source:"),
            text: body.trim().to_owned(),
        });
    }
    notices
}

/// Rocket's two license texts, with Windows line endings removed.
fn licenses() -> &'static (String, String) {
    static TEXTS: OnceLock<(String, String)> = OnceLock::new();
    TEXTS.get_or_init(|| (LICENSE_MIT.replace("\r\n", "\n"), LICENSE_APACHE.replace("\r\n", "\n")))
}

/// "2026 The Rocket contributors", from LICENSE-MIT's copyright line.
pub(crate) fn copyright() -> &'static str {
    LICENSE_MIT.lines().find_map(|l| l.strip_prefix("Copyright (c) ")).unwrap_or_default().trim()
}

/// "MIT or Apache-2.0", from the license in Cargo.toml.
pub(crate) fn license() -> String {
    env!("CARGO_PKG_LICENSE").replace(" OR ", " or ")
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Section {
    #[default]
    Rocket,
    Crates,
    Fonts,
    Runtime,
}

#[derive(Clone, Default)]
struct State {
    section: Section,
    apache: bool,
    filter: String,
    /// The chosen crate's name and version.
    chosen: Option<(String, String)>,
    font: usize,
}

/// Opens Legal at Rocket's license with nothing chosen.
pub(crate) fn reset(ctx: &Context) {
    ctx.data_mut(|d| d.remove::<State>(Id::unique(STATE)));
}

/// Legal: Rocket's license, then what Rocket includes, each with its notice.
pub(crate) fn dialog(ctx: &Context, widgets: &mut HashMap<String, UiControl>) -> pages::ReviewResponse {
    let head = pages::ReviewHeader {
        image: Icon::File,
        title: crate::i18n::tr("Legal"),
        subtitle: crate::i18n::tr("Licenses for Rocket and the open-source software it includes."),
        danger: false,
    };
    let height = (ctx.content_rect().height() - 300.0).clamp(240.0, 520.0);
    pages::full_info(ctx, "rocket-dialog", head, widgets, "dialog-scroll", |ui, part, widgets| {
        if part != pages::Part::Body {
            return;
        }
        let id = Id::unique(STATE);
        let mut s: State = ui.data(|d| d.get_temp(id)).unwrap_or_default();
        let all = notices();
        let count = |kind: &str| all.iter().filter(|n| n.kind == kind).count();
        let runtime = all.iter().find(|n| n.kind == "runtime");
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 16.0;
            ui.vertical(|ui| {
                ui.set_width(220.0);
                ui.spacing_mut().item_spacing.y = 2.0;
                let rows = [
                    (Section::Rocket, "Rocket".to_owned(), license(), "dialog-legal-rocket"),
                    (Section::Crates, crate::i18n::tr("Rust crates").to_owned(), crate::i18n::trf!("Packages: {count}", count = count("crate")), "dialog-legal-crates"),
                    (Section::Fonts, crate::i18n::tr("Fonts").to_owned(), crate::i18n::trf!("Fonts: {count}", count = count("font")), "dialog-legal-fonts"),
                    (
                        Section::Runtime,
                        runtime.map_or_else(String::new, |n| n.name.clone()),
                        runtime.map_or_else(String::new, |n| crate::i18n::trf!("Terminal runtime · {license}", license = n.license)),
                        "dialog-legal-runtime",
                    ),
                ];
                for (i, (section, title, sub, key)) in rows.into_iter().enumerate() {
                    // Linux/macOS bundle no runtime, so there is nothing to list.
                    if section == Section::Runtime && runtime.is_none() {
                        continue;
                    }
                    if i == 1 {
                        ui.add_space(10.0);
                        ui.label(RichText::new(crate::i18n::tr("INCLUDED IN ROCKET")).font(semibold(11.0)).color(HINT()));
                        ui.add_space(2.0);
                    }
                    let r = nav_row(ui, &title, &sub, s.section == section);
                    control(widgets, key, &r, ui.clip_rect());
                    if r.clicked() {
                        s.section = section;
                    }
                }
            });
            let (line, _) = ui.allocate_exact_size(vec2(1.0, height), Sense::hover());
            ui.painter().vline(line.center().x, line.y_range(), Stroke::new(1.0, HAIRLINE()));
            ui.vertical(|ui| {
                ui.set_width(ui.available_width());
                let top = ui.cursor().top();
                let left = |ui: &Ui| (height - (ui.cursor().top() - top)).max(120.0);
                match s.section {
                    Section::Rocket => {
                        heading(ui, "Rocket");
                        ui.add(Label::new(crate::i18n::tr("Rocket is free and open-source software. You may use, copy, change and share it under either license below, at your option.")).wrap());
                        ui.label(RichText::new(format!("Copyright © {}", copyright())).color(MUTED()));
                        ui.add_space(2.0);
                        let items = [(crate::i18n::tr("MIT License"), !s.apache), (crate::i18n::tr("Apache License 2.0"), s.apache)];
                        ui.horizontal(|ui| {
                            for (i, r) in pages::segments(ui, &items).iter().enumerate() {
                                control(widgets, ["dialog-legal-mit", "dialog-legal-apache"][i], r, ui.clip_rect());
                                if r.clicked() {
                                    s.apache = i == 1;
                                }
                            }
                        });
                        let (mit, apache) = licenses();
                        let text = if s.apache { apache } else { mit };
                        text_box(ui, widgets, ("rocket", usize::from(s.apache)), text, left(ui));
                    }
                    Section::Crates => crates(ui, widgets, &mut s, height, top),
                    Section::Fonts => {
                        heading(ui, crate::i18n::tr("Fonts"));
                        ui.add(Label::new(crate::i18n::tr("Rocket includes these fonts for text, code and symbols.")).wrap());
                        let fonts: Vec<&Notice> = all.iter().filter(|n| n.kind == "font").collect();
                        s.font = s.font.min(fonts.len().saturating_sub(1));
                        list_card(ui, |ui| {
                            for (i, font) in fonts.iter().enumerate() {
                                if i > 0 {
                                    crate::preferences::divider(ui);
                                }
                                let r = item_row(ui, &font.name, "", &font.license, i == s.font);
                                control(widgets, format!("dialog-legal-font-{i}"), &r, ui.clip_rect());
                                if r.clicked() {
                                    s.font = i;
                                }
                            }
                        });
                        if let Some(font) = fonts.get(s.font) {
                            text_box(ui, widgets, ("font", s.font), &font.text, left(ui));
                        }
                    }
                    Section::Runtime => {
                        if let Some(n) = runtime {
                            heading(ui, &format!("{} {}", n.name, n.version));
                            ui.add(Label::new(crate::i18n::tr("conpty.dll and OpenConsole.exe run Rocket's terminal. They are unmodified from Microsoft's NuGet package.")).wrap());
                            text_box(ui, widgets, ("runtime", 0), &n.text, left(ui));
                        }
                    }
                }
            });
        });
        ui.data_mut(|d| d.insert_temp(id, s));
    })
}

/// Rust crates: a filter, the crates, then the chosen one's notice.
fn crates(ui: &mut Ui, widgets: &mut HashMap<String, UiControl>, s: &mut State, height: f32, top: f32) {
    let all: Vec<&Notice> = notices().iter().filter(|n| n.kind == "crate").collect();
    ui.horizontal(|ui| {
        heading(ui, crate::i18n::tr("Rust crates"));
        ui.label(RichText::new(crate::i18n::trf!("Packages: {count}", count = all.len())).color(HINT()));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let r = search_field(ui, &mut s.filter, crate::i18n::tr("Find a crate or license"), 240.0);
            control(widgets, "dialog-legal-filter", &r, ui.clip_rect());
        });
    });
    let needle = s.filter.trim().to_lowercase();
    let shown: Vec<&Notice> = all
        .into_iter()
        .filter(|n| needle.is_empty() || n.name.to_lowercase().contains(&needle) || n.license.to_lowercase().contains(&needle))
        .collect();
    if shown.is_empty() {
        ui.label(RichText::new(crate::i18n::tr("No crate matches.")).color(HINT()));
        return;
    }
    let chosen = shown
        .iter()
        .position(|n| s.chosen.as_ref().is_some_and(|(name, version)| *name == n.name && *version == n.version))
        .unwrap_or(0);
    let list_height = ((height - (ui.cursor().top() - top)) * 0.45).max(96.0);
    let list = list_card(ui, |ui| {
        ScrollArea::vertical()
            .id_salt("legal-crates")
            .max_height(list_height)
            .auto_shrink([false, true])
            .show_rows(ui, 32.0, shown.len(), |ui, range| {
                for i in range {
                    let n = shown[i];
                    let r = item_row(ui, &n.name, &n.version, &n.license, i == chosen);
                    control(widgets, format!("dialog-legal-crate-{i}"), &r, ui.clip_rect());
                    if r.clicked() {
                        s.chosen = Some((n.name.clone(), n.version.clone()));
                    }
                }
            })
            .inner_rect
    });
    pages::region(widgets, "dialog-legal-crates-scroll", list.intersect(ui.clip_rect()), true);
    let n = shown[chosen];
    ui.add_space(2.0);
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(format!("{} {}", n.name, n.version)).font(semibold(13.0)));
        ui.label(RichText::new(&n.license).size(12.0).color(HINT()));
        if !n.source.is_empty() {
            ui.label(RichText::new(&n.source).size(12.0).color(FAINT()));
        }
    });
    let left = (height - (ui.cursor().top() - top)).max(96.0);
    text_box(ui, widgets, ("crate", chosen), &n.text, left);
}

fn heading(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).font(semibold(15.0)));
}

/// A section in the list: its name, then what it covers.
fn nav_row(ui: &mut Ui, title: &str, sub: &str, selected: bool) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 44.0), Sense::click());
    let painter = ui.painter();
    if selected {
        painter.rect_filled(rect, 7.0, SELECTED());
        painter.rect_filled(
            Rect::from_min_size(rect.min + vec2(0.0, 9.0), vec2(2.5, rect.height() - 18.0)),
            1.0,
            BLUE(),
        );
    } else if response.hovered() || response.has_focus() {
        painter.rect_filled(rect, 7.0, HOVER());
    }
    let inner = rect.shrink2(vec2(10.0, 0.0));
    clipped_text(ui, Rect::from_min_max(inner.min, pos2(inner.right(), inner.top() + 24.0)), title, 13.0, TEXT());
    clipped_text(ui, Rect::from_min_max(pos2(inner.left(), inner.top() + 22.0), inner.max), sub, 12.0, HINT());
    response.widget_info(|| WidgetInfo::selected(Role::Button, true, selected, title));
    response
}

/// A row in a card: a name and version on the left, the license on the right.
fn item_row(ui: &mut Ui, name: &str, version: &str, license: &str, selected: bool) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 32.0), Sense::click());
    let painter = ui.painter();
    if selected {
        painter.rect_filled(rect, 4.0, SELECTED());
    } else if response.hovered() || response.has_focus() {
        painter.rect_filled(rect, 4.0, HOVER());
    }
    let inner = rect.shrink2(vec2(14.0, 0.0));
    let license = painter.layout_no_wrap(license.to_owned(), FontId::proportional(12.0), MUTED());
    let license_width = license.size().x;
    painter.galley(pos2(inner.right() - license_width, inner.center().y - license.size().y / 2.0), license, MUTED());
    let name_width = (inner.width() - license_width - 16.0).max(40.0);
    let mut job = text::LayoutJob::default();
    job.append(name, 0.0, TextFormat::simple(FontId::proportional(13.0), TEXT()));
    if !version.is_empty() {
        job.append(&format!("  {version}"), 0.0, TextFormat::simple(FontId::monospace(11.5), HINT()));
    }
    job.wrap = text::TextWrapping::truncate_at_width(name_width);
    let galley = painter.layout_job(job);
    painter.galley(pos2(inner.left(), inner.center().y - galley.size().y / 2.0), galley, TEXT());
    response.widget_info(|| WidgetInfo::selected(Role::Button, true, selected, name));
    response
}

/// The rows of a list, on a card.
fn list_card<R>(ui: &mut Ui, content: impl FnOnce(&mut Ui) -> R) -> R {
    Frame::new()
        .fill(CARD())
        .stroke(Stroke::new(1.0, CARD_STROKE()))
        .corner_radius(8)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            content(ui)
        })
        .inner
}

/// A license text in a monospace box that scrolls.
fn text_box(ui: &mut Ui, widgets: &mut HashMap<String, UiControl>, salt: (&str, usize), text: &str, height: f32) {
    let scroll = Frame::new()
        .fill(BG())
        .stroke(Stroke::new(1.0, HAIRLINE()))
        .corner_radius(8)
        .inner_margin(Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ScrollArea::vertical()
                .id_salt(("legal-text", salt))
                .max_height((height - 22.0).max(60.0))
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.add(Label::new(RichText::new(text).font(FontId::monospace(12.0)).color(TEXT_SOFT())).wrap());
                })
                .inner_rect
        })
        .inner;
    pages::region(widgets, "dialog-legal-text-scroll", scroll.intersect(ui.clip_rect()), true);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The notices name each crate at a version Cargo.lock has, and each entry the
    /// platforms whose builds include it. A build lists its own platform's entries:
    /// egui's fonts everywhere, the ConPTY runtime on Windows and spellbook, with its
    /// MPL-2.0 text and where its source is, on Linux. A dependency change without
    /// regenerating fails here.
    #[test]
    fn notices_match_the_locked_crates() {
        let lock = include_str!("../../../Cargo.lock").replace("\r\n", "\n");
        let every = parse(NOTICES);
        for n in &every {
            let named = n.platforms.split(", ").all(|p| ["Windows", "Linux", "macOS"].contains(&p));
            assert!(named, "{} {} names its platforms, not {:?}", n.name, n.version, n.platforms);
            assert!(!n.license.is_empty() && !n.text.is_empty(), "{} has a license and its text", n.name);
            if n.kind == "crate" {
                assert!(
                    lock.contains(&format!("name = \"{}\"\nversion = \"{}\"", n.name, n.version)),
                    "{} {} is not in Cargo.lock; regenerate THIRD-PARTY-NOTICES.txt",
                    n.name,
                    n.version
                );
            }
        }
        let all = notices();
        let crates: Vec<&Notice> = all.iter().filter(|n| n.kind == "crate").collect();
        assert!(crates.len() > 50, "{} crates", crates.len());
        let unix_fonts = PLATFORM.is_none_or(|p| p == "Linux" || p == "macOS");
        assert_eq!(all.iter().filter(|n| n.kind == "font").count(), if unix_fonts { 6 } else { 4 });
        let cjk = every.iter().find(|n| n.name == "Noto Sans CJK Regular").expect("CJK notice");
        assert_eq!((cjk.version.as_str(), cjk.license.as_str(), cjk.platforms.as_str()), ("2.004", "OFL-1.1", "Linux, macOS"));
        assert!(cjk.text.contains("SIL OPEN FONT LICENSE"));
        assert_eq!(all.iter().any(|n| n.name == cjk.name), unix_fonts);
        let emoji = every.iter().find(|n| n.name == "Noto Emoji outline").expect("Emoji fallback notice");
        assert_eq!((emoji.version.as_str(), emoji.license.as_str(), emoji.platforms.as_str()), ("3.002", "OFL-1.1", "Linux, macOS"));
        assert!(emoji.text.contains("SIL OPEN FONT LICENSE"));
        assert_eq!(all.iter().any(|n| n.name == emoji.name), unix_fonts);
        let conpty = all.iter().any(|n| n.kind == "runtime" && n.name == "Microsoft ConPTY" && n.license == "MIT");
        assert_eq!(conpty, PLATFORM.is_none_or(|p| p == "Windows"));
        assert!(crates.iter().any(|n| n.name == "egui" && n.text.contains("Permission is hereby granted")));
        let spellbook = every.iter().find(|n| n.name == "spellbook").expect("spellbook's notice");
        assert_eq!((spellbook.license.as_str(), spellbook.platforms.as_str()), ("MPL-2.0", "Linux"));
        assert!(spellbook.text.contains("Mozilla Public License Version 2.0"));
        assert!(spellbook.text.contains(&format!("https://crates.io/crates/spellbook/{}", spellbook.version)));
        assert_eq!(crates.iter().any(|n| n.name == "spellbook"), PLATFORM.is_none_or(|p| p == "Linux"));
    }

    #[test]
    fn rocket_names_its_license_and_holder() {
        assert_eq!(copyright(), "2026 The Rocket contributors");
        assert_eq!(license(), "MIT or Apache-2.0");
        assert!(licenses().1.contains("Apache License") && licenses().1.contains("END OF TERMS AND CONDITIONS"));
    }

    /// About shows the license and opens Legal; each section opens and × closes it. Linux
    /// and macOS builds bundle no runtime, so Legal has no runtime section there.
    #[test]
    fn about_opens_legal_and_its_sections() {
        crate::test_common::isolated(
            "ROCKET_LEGAL_UI_CASE",
            "legal::tests::about_opens_legal_and_its_sections",
            || {
                use crate::{
                    app::{DialogKind, Rocket},
                    batch::flow_tests::{click, enabled, wait},
                };
                let fixture = crate::test_common::Fixture::new();
                let ctx = Context::default();
                let mut app = Rocket::with_log_directory(&ctx, vec![], fixture.root.join("logs"));
                wait(&ctx, &mut app);
                app.show_dialog(DialogKind::About);
                assert!(enabled(&ctx, &mut app, "dialog-about-legal"));
                click(&ctx, &mut app, "dialog-about-legal");
                assert!(matches!(app.dialog.as_ref().map(|d| &d.kind), Some(DialogKind::Legal)));
                assert!(enabled(&ctx, &mut app, "dialog-legal-mit"));
                click(&ctx, &mut app, "dialog-legal-apache");
                click(&ctx, &mut app, "dialog-legal-crates");
                assert!(enabled(&ctx, &mut app, "dialog-legal-crate-1"));
                click(&ctx, &mut app, "dialog-legal-crate-1");
                assert!(enabled(&ctx, &mut app, "dialog-legal-filter"));
                click(&ctx, &mut app, "dialog-legal-fonts");
                assert!(enabled(&ctx, &mut app, "dialog-legal-font-3"));
                if cfg!(any(target_os = "linux", target_os = "macos")) {
                    assert!(enabled(&ctx, &mut app, "dialog-legal-font-4"));
                    click(&ctx, &mut app, "dialog-legal-font-4");
                    assert!(enabled(&ctx, &mut app, "dialog-legal-font-5"));
                    click(&ctx, &mut app, "dialog-legal-font-5");
                    assert!(!app.frame_widgets.contains_key("dialog-legal-runtime"));
                } else {
                    click(&ctx, &mut app, "dialog-legal-runtime");
                }
                click(&ctx, &mut app, "dialog-close");
                assert!(app.dialog.is_none());
            },
        );
    }
}
